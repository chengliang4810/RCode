//! Narrow, owned C ABI for Flutter's portable visual renderer.
//!
//! Dart polls coalesced display lists and takes pixel buffers explicitly. No
//! worker calls into a Dart isolate, and no network/GPU work runs in an FFI call.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::layout::{FaceRole, PlatformMeasurer, TextSystem};
use crate::visual::{
    PointerPhase, SurfaceFrame, VisualDisplay, VisualHub, VisualListener, VisualSlot,
};

const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 128 * 1024;

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

#[derive(Default)]
struct Pending {
    display: Option<VisualDisplay>,
    frames: BTreeMap<u64, SurfaceFrame>,
    measurements: BTreeMap<u64, Value>,
    measured: HashMap<u64, Vec<f32>>,
    buffers: HashMap<u64, Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    measured: Condvar,
    sequence: AtomicU64,
    closed: AtomicBool,
}

impl VisualListener for Shared {
    fn display(&self, display: VisualDisplay) {
        lock(&self.pending).display = Some(display);
    }
    fn frame(&self, frame: SurfaceFrame) {
        lock(&self.pending).frames.insert(frame.surface, frame);
    }
}

impl PlatformMeasurer for Shared {
    fn measure(&self, face: FaceRole, size: f32, ligatures: bool, text: String) -> f32 {
        self.measure_run(face, size, ligatures, text).iter().sum()
    }

    fn measure_run(&self, face: FaceRole, size: f32, ligatures: bool, text: String) -> Vec<f32> {
        if self.closed.load(Ordering::Acquire) {
            return vec![];
        }
        let id = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let mut pending = lock(&self.pending);
        pending.measurements.insert(
            id,
            json!({"id":id,"face":face,"size":size,"ligatures":ligatures,"text":text}),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.closed.load(Ordering::Acquire) {
            if let Some(result) = pending.measured.remove(&id) {
                return result;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            pending = self
                .measured
                .wait_timeout(pending, remaining)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
        pending.measurements.remove(&id);
        vec![]
    }
}

struct Files {
    endpoint: String,
    api_key: String,
    device: String,
    client: Mutex<Option<Arc<rcode_rpc::RpcClient>>>,
    closed: AtomicBool,
}

impl Files {
    fn fetch(&self, path: &str) -> Option<Vec<u8>> {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        // Invoked by the visual's worker, never by the Flutter isolate.
        rcode_client::runtime::shared().block_on(async {
            let existing = lock(&self.client).clone();
            let client = match existing {
                Some(client) => client,
                None => {
                    let client = Arc::new(
                        tokio::time::timeout(
                            Duration::from_secs(8),
                            rcode_rpc::connect_ws_authenticated(&self.endpoint, &self.api_key),
                        )
                        .await
                        .ok()?
                        .ok()?,
                    );
                    if self.closed.load(Ordering::Acquire) {
                        client.close();
                        return None;
                    }
                    *lock(&self.client) = Some(client.clone());
                    client
                }
            };
            tokio::time::timeout(Duration::from_secs(20), async {
                let mut bytes = Vec::new();
                let mut offset = 0_u64;
                loop {
                    if self.closed.load(Ordering::Acquire) {
                        return None;
                    }
                    let frame = client
                        .call(
                            rcode_rpc::methods::READ_ATTACHMENT_CHUNK,
                            json!({"targetDeviceId":self.device,"path":path,"offset":offset}),
                        )
                        .await
                        .ok()?;
                    let chunk = base64::engine::general_purpose::STANDARD
                        .decode(frame["data"].as_str()?)
                        .ok()?;
                    if bytes.len().saturating_add(chunk.len()) > MAX_FILE_BYTES {
                        return None;
                    }
                    bytes.extend(chunk);
                    if frame["done"].as_bool()? {
                        return Some(bytes);
                    }
                    let next = frame["nextOffset"].as_u64()?;
                    if next <= offset {
                        return None;
                    }
                    offset = next;
                }
            })
            .await
            .ok()
            .flatten()
        })
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(client) = lock(&self.client).take() {
            client.close();
        }
    }
}

struct Instance {
    hub: VisualHub,
    session: Arc<crate::visual::VisualSession>,
    slot: Arc<VisualSlot>,
    shared: Arc<Shared>,
    files: Arc<Files>,
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.measured.notify_all();
        self.files.close();
        self.hub.shutdown();
    }
}

static INSTANCES: OnceLock<Mutex<HashMap<u64, Arc<Instance>>>> = OnceLock::new();
static FONTS: OnceLock<Mutex<HashMap<u8, &'static [u8]>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
static ERROR: Mutex<String> = Mutex::new(String::new());

fn instances() -> &'static Mutex<HashMap<u64, Arc<Instance>>> {
    INSTANCES.get_or_init(Mutex::default)
}
fn fonts() -> &'static Mutex<HashMap<u8, &'static [u8]>> {
    FONTS.get_or_init(Mutex::default)
}
fn instance(id: u64) -> Option<Arc<Instance>> {
    lock(instances()).get(&id).cloned()
}
fn fail(error: &str) {
    *lock(&ERROR) = error.to_owned();
}
fn boundary<T: Default>(work: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
        fail("Native visual renderer failed");
        T::default()
    })
}
fn role(index: u8) -> Option<FaceRole> {
    use FaceRole::*;
    [
        Sans,
        SansMedium,
        SansSemibold,
        SansBold,
        SansItalic,
        SansMediumItalic,
        SansSemiboldItalic,
        SansBoldItalic,
        Mono,
        MonoMedium,
        MonoSemibold,
        MonoItalic,
    ]
    .get(index as usize)
    .copied()
}

unsafe fn input(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // Callers own a valid NUL-terminated UTF-8 allocation for the duration of the call.
    let bytes = unsafe { CStr::from_ptr(pointer) }.to_bytes();
    if bytes.len() > MAX_JSON_BYTES {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}
fn string(value: &impl serde::Serialize) -> *mut c_char {
    serde_json::to_string(value)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map_or(std::ptr::null_mut(), CString::into_raw)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    endpoint: String,
    api_key: String,
    device_id: String,
    path: String,
    id: String,
    #[serde(default)]
    title: String,
    width: f32,
    #[serde(default = "one")]
    text_scale: f32,
    #[serde(default = "one")]
    pixel_scale: f32,
    #[serde(default)]
    dark: bool,
    #[serde(default)]
    reduce_motion: bool,
    #[serde(default)]
    locale: String,
}

fn chinese(locale: &str) -> bool {
    locale
        .split(['-', '_'])
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("zh"))
}
fn identifier(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn one() -> f32 {
    1.0
}

/// Register each bundled face once, before creating visual sessions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_visual_font(index: u8, data: *const u8, len: usize) -> bool {
    boundary(|| {
        if role(index).is_none() || data.is_null() || !(1..=16 * 1024 * 1024).contains(&len) {
            return false;
        }
        let mut fonts = lock(fonts());
        fonts.entry(index).or_insert_with(|| {
            Box::leak(
                unsafe { std::slice::from_raw_parts(data, len) }
                    .to_vec()
                    .into_boxed_slice(),
            )
        });
        true
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_visual_create(config: *const c_char) -> u64 {
    boundary(|| {
        let Some(config) =
            (unsafe { input(config) }).and_then(|text| serde_json::from_str::<Config>(&text).ok())
        else {
            fail("Invalid visual configuration");
            return 0;
        };
        if config.endpoint.is_empty()
            || config.path.is_empty()
            || !config.width.is_finite()
            || config.width <= 0.0
        {
            fail("Invalid visual source or size");
            return 0;
        }
        let faces = lock(fonts())
            .iter()
            .filter_map(|(index, bytes)| Some((role(*index)?, *bytes)))
            .collect();
        let shared = Arc::new(Shared::default());
        let text = TextSystem::with_static_faces(faces, Some(shared.clone()));
        let files = Arc::new(Files {
            endpoint: config.endpoint,
            api_key: config.api_key,
            device: config.device_id,
            client: Mutex::new(None),
            closed: AtomicBool::new(false),
        });
        let source = files.clone();
        let hub = VisualHub::new(text, Arc::new(|_| {}));
        hub.set_fetch(
            Arc::new(move |path| source.fetch(path)),
            config.pixel_scale.clamp(1.0, 3.0),
        );
        let slot = VisualSlot::new(&config.id, &config.path, &config.title);
        hub.ensure(&slot, 1, config.width, config.text_scale.clamp(0.5, 3.0));
        let session = hub.session(&config.id).expect("visual worker created");
        session.set_locale(chinese(&config.locale));
        session.attach(shared.clone(), config.dark, config.reduce_motion);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        lock(instances()).insert(
            id,
            Arc::new(Instance {
                hub,
                session,
                slot,
                shared,
                files,
            }),
        );
        id
    })
}

/// Coalesced updates. Pixel data is taken separately, avoiding base64 copies.
#[unsafe(no_mangle)]
pub extern "C" fn rcode_visual_poll(id: u64) -> *mut c_char {
    boundary(|| {
        let Some(instance) = instance(id) else {
            return std::ptr::null_mut();
        };
        let mut pending = lock(&instance.shared.pending);
        if pending.display.is_none() && pending.frames.is_empty() && pending.measurements.is_empty()
        {
            return std::ptr::null_mut();
        }
        pending.buffers.clear();
        let display = pending.display.take().map(|display| {
            let mut value = serde_json::to_value(display).expect("serializable display");
            // Node ids are full u64 hashes. JSON numbers above i64::MAX lose
            // precision in Dart; strings preserve every bit in both directions.
            for field in ["surfaces", "controls"] {
                if let Some(rows) = value[field].as_array_mut() {
                    for row in rows {
                        if let Some(id) = row["id"].as_u64() {
                            row["id"] = json!(id.to_string());
                        }
                    }
                }
            }
            value
        });
        let frames = std::mem::take(&mut pending.frames).into_values().map(|frame| {
            let image = frame.image.map(|image| {
                let token = instance.shared.sequence.fetch_add(1, Ordering::Relaxed) + 1;
                pending.buffers.insert(token, image.bgra);
                json!({"token":token,"width":image.width,"height":image.height})
            });
            json!({"surface":frame.surface.to_string(),"image":image,"paint":frame.paint,"styles":frame.styles})
        }).collect::<Vec<_>>();
        let measurements = std::mem::take(&mut pending.measurements)
            .into_values()
            .collect::<Vec<_>>();
        string(&json!({"display":display,"frames":frames,"measurements":measurements}))
    })
}

#[repr(C)]
#[derive(Default)]
pub struct VisualBytes {
    pub data: *mut u8,
    pub len: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn rcode_visual_image(id: u64, token: u64) -> VisualBytes {
    boundary(|| {
        let Some(instance) = instance(id) else {
            return VisualBytes::default();
        };
        let Some(bytes) = lock(&instance.shared.pending).buffers.remove(&token) else {
            return VisualBytes::default();
        };
        let mut bytes = bytes.into_boxed_slice();
        let result = VisualBytes {
            data: bytes.as_mut_ptr(),
            len: bytes.len(),
        };
        std::mem::forget(bytes);
        result
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_visual_bytes_free(bytes: VisualBytes) {
    boundary(|| {
        if !bytes.data.is_null() {
            unsafe {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    bytes.data, bytes.len,
                )));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_visual_command(id: u64, command: *const c_char) {
    boundary(|| {
        let Some(instance) = instance(id) else {
            return;
        };
        let Some(value) =
            (unsafe { input(command) }).and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            return;
        };
        match value["op"].as_str() {
            Some("layout") => {
                let width = value["width"].as_f64().unwrap_or(0.0) as f32;
                let scale = value["textScale"].as_f64().unwrap_or(1.0) as f32;
                if width.is_finite() && width > 0.0 && scale.is_finite() {
                    instance
                        .hub
                        .ensure(&instance.slot, 1, width, scale.clamp(0.5, 3.0));
                }
            }
            Some("attach") => instance.session.attach(
                instance.shared.clone(),
                value["dark"].as_bool().unwrap_or(false),
                value["reduceMotion"].as_bool().unwrap_or(false),
            ),
            Some("detach") => instance.session.detach(),
            Some("appearance") => instance
                .session
                .set_appearance(value["dark"].as_bool().unwrap_or(false)),
            Some("locale") => instance
                .session
                .set_locale(chinese(value["locale"].as_str().unwrap_or("en"))),
            Some("activate") => instance
                .session
                .activate(identifier(&value["control"]).unwrap_or(0)),
            Some("slide") => instance.session.slide(
                identifier(&value["control"]).unwrap_or(0),
                value["fraction"].as_f64().unwrap_or(0.0) as f32,
            ),
            Some("pointer") => instance.session.pointer(
                identifier(&value["surface"]).unwrap_or(0),
                match value["phase"].as_str() {
                    Some("down") => PointerPhase::Down,
                    Some("up") => PointerPhase::Up,
                    Some("cancel") => PointerPhase::Cancel,
                    _ => PointerPhase::Move,
                },
                value["x"].as_f64().unwrap_or(0.0) as f32,
                value["y"].as_f64().unwrap_or(0.0) as f32,
            ),
            Some("measured") => {
                if let Some(request) = value["id"].as_u64() {
                    let advances = value["advances"]
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                                .collect()
                        })
                        .unwrap_or_default();
                    lock(&instance.shared.pending)
                        .measured
                        .insert(request, advances);
                    instance.shared.measured.notify_all();
                }
            }
            _ => {}
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn rcode_visual_dispose(id: u64) {
    boundary(|| {
        lock(instances()).remove(&id);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn rcode_visual_error() -> *mut c_char {
    boundary(|| string(&*lock(&ERROR)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_visual_string_free(pointer: *mut c_char) {
    boundary(|| {
        if !pointer.is_null() {
            unsafe {
                drop(CString::from_raw(pointer));
            }
        }
    });
}

/// The same six bundled math faces as the transcript renderer. SVG contains
/// glyph outlines, so platform text/font fallback cannot change the formula.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcode_math_svg(request: *const c_char) -> *mut c_char {
    boundary(|| {
        let Some(value) =
            (unsafe { input(request) }).and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            return std::ptr::null_mut();
        };
        let Some(source) = value["source"]
            .as_str()
            .filter(|source| source.len() <= 4096)
        else {
            return std::ptr::null_mut();
        };
        let size = value["fontSize"].as_f64().unwrap_or(16.0) as f32;
        if !size.is_finite() || !(6.0..=128.0).contains(&size) {
            return std::ptr::null_mut();
        }
        use zatex::bundled::Face;
        let face = match value["font"].as_str() {
            Some("stixTwo") => Face::StixTwo,
            Some("computerModern") => Face::ComputerModern,
            Some("libertinus") => Face::Libertinus,
            Some("pagella") => Face::Pagella,
            Some("euler") => Face::Euler,
            _ => Face::Fira,
        };
        let options = zatex::Options {
            size,
            display: value["display"].as_bool().unwrap_or(false),
            max_bytes: 4096,
            max_depth: 32,
            max_items: 4096,
            max_width: 200.0,
            max_height: 50.0,
        };
        let Ok(formula) = zatex::bundled::layout(source, face, &options) else {
            return std::ptr::null_mut();
        };
        let svg = formula
            .to_svg(zatex::bundled::font(face))
            .replacen(
                &format!("width=\"{:.2}\"", formula.width),
                &format!("width=\"{:.2}\"", formula.span()),
                1,
            )
            .replacen(
                &format!(
                    "viewBox=\"0 0 {:.2} {:.2}\"",
                    formula.width,
                    formula.height()
                ),
                &format!(
                    "viewBox=\"{:.2} 0 {:.2} {:.2}\"",
                    formula.left,
                    formula.span(),
                    formula.height()
                ),
                1,
            );
        string(
            &json!({"svg":svg,"width":formula.span(),"height":formula.height(),"ascent":formula.ascent}),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_fixture(script: &str) -> (u64, std::sync::Weak<Shared>) {
        let shared = Arc::new(Shared::default());
        let files = Arc::new(Files {
            endpoint: String::new(),
            api_key: String::new(),
            device: String::new(),
            client: Mutex::new(None),
            closed: AtomicBool::new(false),
        });
        let hub = VisualHub::new(crate::layout::tests::text_system(), Arc::new(|_| {}));
        let script = script.as_bytes().to_vec();
        hub.set_fetch(
            Arc::new(move |path| (path == "fixture-visual.luau").then(|| script.clone())),
            1.0,
        );
        let slot = VisualSlot::new("fixture", "fixture-visual.luau", "Fixture");
        hub.ensure(&slot, 1, 300.0, 1.0);
        let session = hub.session("fixture").unwrap();
        session.attach(shared.clone(), false, true);
        let weak = Arc::downgrade(&shared);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        lock(instances()).insert(
            id,
            Arc::new(Instance {
                hub,
                session,
                slot,
                shared,
                files,
            }),
        );
        (id, weak)
    }

    fn poll(id: u64) -> Option<Value> {
        let pointer = rcode_visual_poll(id);
        if pointer.is_null() {
            return None;
        }
        unsafe {
            let value = serde_json::from_str(CStr::from_ptr(pointer).to_str().unwrap()).unwrap();
            rcode_visual_string_free(pointer);
            Some(value)
        }
    }

    fn command(id: u64, value: Value) {
        let text = CString::new(value.to_string()).unwrap();
        unsafe {
            rcode_visual_command(id, text.as_ptr());
        }
    }

    fn display(id: u64, text: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(value) = poll(id)
                && value["display"]["paint"]["text"]
                    .as_str()
                    .is_some_and(|value| value.contains(text))
            {
                return value["display"].clone();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("visual display did not reach {text}");
    }

    #[test]
    fn c_abi_runs_real_luau_controls_and_parallel_poll_disposes_its_worker() {
        let (id, shared) = open_fixture(
            r#"
          local s = ui.state { n = 0 }
          return function() return ui.column {
            ui.text("Count " .. s.n),
            ui.button {key="native-control", label="Add", on_click=function() s.n=s.n+1 end},
          } end
        "#,
        );
        let initial = display(id, "Count 0");
        let control = initial["controls"][0]["id"].clone();
        assert!(identifier(&control).unwrap() > i64::MAX as u64);
        command(id, json!({"op":"activate","control":control}));
        display(id, "Count 1");
        command(id, json!({"op":"locale","locale":"zh-Hans"}));
        display(id, "Count 1");
        command(id, json!({"op":"locale","locale":"en"}));
        display(id, "Count 1");
        let polling = std::thread::spawn(move || {
            for _ in 0..200 {
                let _ = poll(id);
            }
        });
        for width in 300..330 {
            command(id, json!({"op":"layout","width":width,"textScale":1}));
        }
        rcode_visual_dispose(id);
        polling.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while shared.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            shared.upgrade().is_none(),
            "disposing the C handle ends its worker and listener"
        );
        assert!(rcode_visual_poll(id).is_null());
    }

    #[test]
    fn c_abi_pixel_buffer_survives_the_visual_until_explicit_release() {
        let (id, _) = open_fixture("return ui.text 'Pixels'");
        display(id, "Pixels");
        let instance = instance(id).unwrap();
        instance.shared.frame(SurfaceFrame {
            surface: 42,
            image: Some(crate::visual::FrameImage {
                width: 2,
                height: 1,
                bgra: vec![1, 2, 3, 255, 4, 5, 6, 255],
            }),
            paint: Default::default(),
            styles: vec![],
        });
        let update = poll(id).unwrap();
        let token = update["frames"][0]["image"]["token"].as_u64().unwrap();
        let pixels = rcode_visual_image(id, token);
        assert_eq!(pixels.len, 8);
        assert!(
            rcode_visual_image(id, token).data.is_null(),
            "each buffer transfers once"
        );
        rcode_visual_dispose(id);
        drop(instance);
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(pixels.data, pixels.len),
                &[1, 2, 3, 255, 4, 5, 6, 255]
            );
            rcode_visual_bytes_free(pixels);
        }
    }

    #[test]
    fn c_abi_rejects_invalid_config_and_unknown_handles_without_unwinding() {
        unsafe {
            assert_eq!(rcode_visual_create(std::ptr::null()), 0);
            let error = rcode_visual_error();
            assert!(!error.is_null());
            assert!(!CStr::from_ptr(error).to_str().unwrap().contains("apiKey"));
            rcode_visual_string_free(error);
            assert!(rcode_visual_poll(u64::MAX).is_null());
            let bytes = rcode_visual_image(u64::MAX, 1);
            assert!(bytes.data.is_null());
            rcode_visual_bytes_free(bytes);
            rcode_visual_command(u64::MAX, std::ptr::null());
            rcode_visual_dispose(u64::MAX);
        }
    }

    #[test]
    fn math_svg_c_abi_preserves_all_six_faces_and_rejects_incomplete_formulas() {
        let mut shapes = std::collections::HashSet::new();
        for font in [
            "fira",
            "stixTwo",
            "computerModern",
            "libertinus",
            "pagella",
            "euler",
        ] {
            let request = CString::new(
                json!({"source":"\\frac{x^2}{2}","font":font,"fontSize":18,"display":true})
                    .to_string(),
            )
            .unwrap();
            unsafe {
                let pointer = rcode_math_svg(request.as_ptr());
                assert!(!pointer.is_null(), "{font}");
                let value: Value =
                    serde_json::from_str(CStr::from_ptr(pointer).to_str().unwrap()).unwrap();
                assert!(value["width"].as_f64().unwrap() > 0.0);
                assert!(value["height"].as_f64().unwrap() > value["ascent"].as_f64().unwrap());
                shapes.insert(value["svg"].as_str().unwrap().to_owned());
                rcode_visual_string_free(pointer);
            }
        }
        assert_eq!(shapes.len(), 6);
        let incomplete = CString::new(r#"{"source":"\\frac{","fontSize":16}"#).unwrap();
        assert!(unsafe { rcode_math_svg(incomplete.as_ptr()) }.is_null());
    }
}
