//! A thin wrapper around `pw_filter`, which pipewire-rs does not cover yet.

use std::ffi::{CString, c_void};
use std::ptr;

use pipewire as pw;
use pw::properties::properties;
use pw::spa::sys as spa_sys;
use pw::sys as pw_sys;

use super::rt::Processor;

const SCRATCH: usize = 16384;

struct Data {
    processor: Processor,
    input: *mut c_void,
    out: [*mut c_void; 2],
    monitor: [*mut c_void; 2],
    scratch: [Vec<f32>; 2],
}

pub struct Filter {
    ptr: *mut pw_sys::pw_filter,
    data: *mut Data,
    _hook: Box<spa_sys::spa_hook>,
    _events: Box<pw_sys::pw_filter_events>,
}

unsafe extern "C" fn on_process(data: *mut c_void, position: *mut spa_sys::spa_io_position) {
    if data.is_null() || position.is_null() {
        return;
    }
    // SAFETY: `data` is the `Data` box this filter owns. PipeWire calls process from one
    // thread at a time, and nothing else touches it while the filter is connected.
    let d = unsafe { &mut *(data as *mut Data) };
    let pos = unsafe { &*position };
    let n = (pos.clock.duration as usize).min(SCRATCH) as u32;
    let rate = pos.clock.rate.denom.max(1);
    if n == 0 {
        return;
    }
    let buf = |port: *mut c_void| unsafe { pw_sys::pw_filter_get_dsp_buffer(port, n) as *mut f32 };
    let n = n as usize;
    let input = buf(d.input);
    let input = (!input.is_null()).then(|| unsafe { std::slice::from_raw_parts(input, n) });

    let [s0, s1] = &mut d.scratch;
    let ol = buf(d.out[0]);
    let or = buf(d.out[1]);
    let out_l = if ol.is_null() { &mut s0[..n] } else { unsafe { std::slice::from_raw_parts_mut(ol, n) } };
    let out_r = if or.is_null() { &mut s1[..n] } else { unsafe { std::slice::from_raw_parts_mut(or, n) } };

    let ml = buf(d.monitor[0]);
    let mr = buf(d.monitor[1]);
    let monitor = if ml.is_null() || mr.is_null() {
        None
    } else {
        unsafe { Some([std::slice::from_raw_parts_mut(ml, n), std::slice::from_raw_parts_mut(mr, n)]) }
    };
    d.processor.process(rate, input, [out_l, out_r], monitor);
}

impl Filter {
    pub fn new(core: &pw::core::Core, processor: Processor, latency: &str) -> anyhow::Result<Self> {
        let props = properties! {
            "media.type" => "Audio",
            "media.category" => "Filter",
            "media.role" => "DSP",
            "node.name" => super::ENGINE_NODE,
            "node.description" => "Voicenux voice changer",
            "node.latency" => latency,
            "node.always-process" => "true",
            "node.want-driver" => "true",
            "application.name" => "Voicenux",
        };
        let name = CString::new("voicenux")?;
        // SAFETY: plain FFI. The properties are handed over to PipeWire.
        let ptr = unsafe { pw_sys::pw_filter_new(core.as_raw_ptr(), name.as_ptr(), props.into_raw()) };
        if ptr.is_null() {
            anyhow::bail!("PipeWire could not create the filter");
        }
        let data = Box::into_raw(Box::new(Data {
            processor,
            input: ptr::null_mut(),
            out: [ptr::null_mut(); 2],
            monitor: [ptr::null_mut(); 2],
            scratch: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
        }));
        let mut events: Box<pw_sys::pw_filter_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw_sys::PW_VERSION_FILTER_EVENTS;
        events.process = Some(on_process);
        let mut hook: Box<spa_sys::spa_hook> = Box::new(unsafe { std::mem::zeroed() });
        unsafe {
            pw_sys::pw_filter_add_listener(ptr, &mut *hook, &*events, data as *mut c_void);
        }

        let port = |dir: spa_sys::spa_direction, name: &str, channel: &str| -> *mut c_void {
            let p = properties! {
                "format.dsp" => "32 bit float mono audio",
                "port.name" => name,
                "audio.channel" => channel,
            };
            unsafe {
                pw_sys::pw_filter_add_port(
                    ptr,
                    dir,
                    pw_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                    8,
                    p.into_raw(),
                    ptr::null_mut(),
                    0,
                )
            }
        };
        let input = spa_sys::SPA_DIRECTION_INPUT;
        let output = spa_sys::SPA_DIRECTION_OUTPUT;
        unsafe {
            (*data).input = port(input, "input_MONO", "MONO");
            (*data).out = [port(output, "output_FL", "FL"), port(output, "output_FR", "FR")];
            (*data).monitor = [port(output, "monitor_FL", "FL"), port(output, "monitor_FR", "FR")];
        }
        let res = unsafe {
            pw_sys::pw_filter_connect(ptr, pw_sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS, ptr::null_mut(), 0)
        };
        let filter = Self { ptr, data, _hook: hook, _events: events };
        if res < 0 {
            anyhow::bail!("PipeWire refused the filter ({res})");
        }
        Ok(filter)
    }

    pub fn set_latency(&self, latency: &str) {
        let props = properties! { "node.latency" => latency };
        let dict: &pw::spa::utils::dict::DictRef = props.as_ref();
        unsafe {
            pw_sys::pw_filter_update_properties(self.ptr, ptr::null_mut(), dict.as_raw_ptr());
        }
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        // SAFETY: destroying the filter stops the realtime callbacks before the data goes.
        unsafe {
            pw_sys::pw_filter_destroy(self.ptr);
            drop(Box::from_raw(self.data));
        }
    }
}
