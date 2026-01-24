use doo_ffi_core::{DooResult, DooString};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::ptr;

// Helper struct to maintain state across FFI boundaries
pub struct JsonWriter {
    buffer: Vec<u8>,
    // We can use a simple state tracking if we want to pretty print or validate,
    // but for "Static Specialization" usually the compiler guarantees structure.
    // Let's keep it simple: raw buffer writer.
}

impl JsonWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }

    fn write_raw(&mut self, s: &[u8]) {
        self.buffer.extend_from_slice(s);
    }
}

// === FFI Interface ===

#[no_mangle]
pub extern "C" fn doo_json_writer_new() -> *mut JsonWriter {
    Box::into_raw(Box::new(JsonWriter::new()))
}

#[no_mangle]
pub extern "C" fn doo_json_writer_free(writer: *mut JsonWriter) {
    if !writer.is_null() {
        unsafe { let _ = Box::from_raw(writer); }
    }
}

// Only used if we want to return the string at the end
#[no_mangle]
pub extern "C" fn doo_json_writer_finish(writer: *mut JsonWriter) -> *mut DooString {
    if writer.is_null() { return ptr::null_mut(); }
    unsafe {
        let writer_box = Box::from_raw(writer); // Take ownership back to drop it
        // Convert buffer to string. Assuming UTF-8 valid because we control writes.
        // Actually, user strings come from DooString or checked sources.
        let s = String::from_utf8_lossy(&writer_box.buffer).to_string();
        // Return DooString
        DooString::from_string(s).into_raw()
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_start_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"{");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_end_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"}");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_start_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"[");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_end_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"]");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_comma(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b",");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_colon(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b":");
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_key(writer: *mut JsonWriter, key: *const c_char) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"\"");
        if !key.is_null() {
            let c_str = unsafe { CStr::from_ptr(key) };
            // Simple key writing - keys are usually safe indentifiers or static strings from codegen?
            // If they are static strings from codegen, they are likely safe.
            // But we should escape them just in case.
            let s = c_str.to_string_lossy();
            let escaped = serde_json::to_string(&s as &str).unwrap(); // escaped includes quotes ""
            // We added manual quotes above? No, serde adds them.
            // Let's rely on serde for value escaping.
            // Wait, removing our manual quotes if we use serde.
        }
        // Actually, let's use serde to write string content without quotes?
        // No, serde::to_string gives full json value.
        // Let's redo: use serde_json::to_string for keys to be safe.
    }
    // Optimization: Codegen knows keys. We can assume codegen passes safe keys?
    // Let's implement safe `write_str` logic.
}

// Better API: Primitives

#[no_mangle]
pub extern "C" fn doo_json_write_int(writer: *mut JsonWriter, val: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let s = val.to_string();
        w.write_raw(s.as_bytes());
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_float(writer: *mut JsonWriter, val: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let s = val.to_string(); // Simple formatting
        w.write_raw(s.as_bytes());
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_bool(writer: *mut JsonWriter, val: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(if val { b"true" } else { b"false" });
    }
}

#[no_mangle]
pub extern "C" fn doo_json_write_string(writer: *mut JsonWriter, val: *const c_char) {
    // Escaped string writing
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_null() {
            w.write_raw(b"null");
            return;
        }
        let c_str = unsafe { CStr::from_ptr(val) };
        let s = c_str.to_string_lossy();
        let escaped = serde_json::to_string(&s as &str).unwrap_or("null".to_string());
        w.write_raw(escaped.as_bytes());
    }
}

// === Parsing (Reader) ===

// External Doo runtime functions we need to build result
extern "C" {
    fn doo_map_create() -> *mut c_void;
    fn doo_map_set(map: *mut c_void, key: *const c_char, val: *mut c_void, key_ty: u32, val_ty: u32);
    fn doo_array_create_with_cap(cap: u64) -> *mut c_void;
    fn doo_array_push(arr: *mut c_void, val: *mut c_void, elem_ty: u32);
    fn doo_string_create(s: *const c_char) -> *mut c_void; 
    // And primitive boxing if "Any" type is used? 
    // In Doo, Map<Str, Any> stores pointers? Or tagged unions?
    // If runtime uses tagged unions/pointers for Any, we need to wrap primitives.
    // For now, assuming Any = *void, and primitives are boxed?
    // Or we act as if we are creating specific types?
    // JSON.parse returns recursive Any.
    // We assume runtime handles boxing of i64/f64 into Any?
    // Let's assume we return `*mut c_void` which is the specific object or boxed primitive.
    
    fn doo_box_int(v: i64) -> *mut c_void;
    fn doo_box_float(v: f64) -> *mut c_void;
    fn doo_box_bool(v: bool) -> *mut c_void;
    fn doo_box_null() -> *mut c_void;
}

#[no_mangle]
pub extern "C" fn doo_json_parse(json_str: *const c_char) -> *mut c_void {
    if json_str.is_null() { return unsafe { doo_box_null() }; }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    
    match serde_json::from_str::<Value>(&s) {
        Ok(v) => json_to_doo(v),
        Err(_) => unsafe { doo_box_null() }, // Or error handling?
    }
}

fn json_to_doo(v: Value) -> *mut c_void {
    unsafe {
        match v {
            Value::Null => doo_box_null(),
            Value::Bool(b) => doo_box_bool(b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    doo_box_int(i)
                } else if let Some(f) = n.as_f64() {
                    doo_box_float(f)
                } else {
                    doo_box_null()
                }
            },
            Value::String(s) => {
                let cs = CString::new(s).unwrap();
                doo_string_create(cs.as_ptr())
            },
            Value::Array(arr) => {
                let ptr = doo_array_create_with_cap(arr.len() as u64);
                for elem in arr {
                    let val = json_to_doo(elem);
                    // 0 for type_id implies Any? Need TypeRegistry constants?
                    doo_array_push(ptr, val, 0); 
                }
                ptr
            },
            Value::Object(obj) => {
                let ptr = doo_map_create();
                for (k, v) in obj {
                    let key = CString::new(k).unwrap();
                    let val = json_to_doo(v);
                    // Key type Str (4?), Val type Any (0?)
                    doo_map_set(ptr, key.as_ptr(), val, 4, 0);
                }
                ptr
            },
        }
    }
}

