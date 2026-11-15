//! A small Objective-C bridge without an additional Rust dependency.
//! Called through GPUI's App on the main thread, never from a scroll frame.

use std::ffi::{c_char, c_void};

type Id = *mut c_void;
type Sel = *mut c_void;
#[cfg(target_arch = "aarch64")]
type ObjcBool = bool;
#[cfg(not(target_arch = "aarch64"))]
type ObjcBool = i8;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}

pub(super) fn reduce_motion() -> Result<bool, String> {
    // SAFETY: these AppKit/NSObject selectors have the exact signatures used
    // below. Class and object existence and selector availability are checked
    // before querying. The local pool bounds any autoreleased temporary objects.
    unsafe {
        let send_id: unsafe extern "C" fn(Id, Sel) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let send_bool: unsafe extern "C" fn(Id, Sel) -> ObjcBool =
            std::mem::transmute(objc_msgSend as *const ());
        let responds: unsafe extern "C" fn(Id, Sel, Sel) -> ObjcBool =
            std::mem::transmute(objc_msgSend as *const ());

        let pool_class = objc_getClass(c"NSAutoreleasePool".as_ptr());
        let workspace_class = objc_getClass(c"NSWorkspace".as_ptr());
        if pool_class.is_null() || workspace_class.is_null() {
            return Err("NSWorkspace is unavailable".into());
        }
        let pool = send_id(pool_class, sel_registerName(c"alloc".as_ptr()));
        let pool = send_id(pool, sel_registerName(c"init".as_ptr()));
        if pool.is_null() {
            return Err("could not create AppKit autorelease pool".into());
        }
        let _pool = AutoreleasePool(pool);
        let workspace = send_id(
            workspace_class,
            sel_registerName(c"sharedWorkspace".as_ptr()),
        );
        if workspace.is_null() {
            return Err("NSWorkspace.sharedWorkspace is unavailable".into());
        }
        let selector = sel_registerName(c"accessibilityDisplayShouldReduceMotion".as_ptr());
        if responds(
            workspace,
            sel_registerName(c"respondsToSelector:".as_ptr()),
            selector,
        ) as u8
            == 0
        {
            return Err("NSWorkspace reduce-motion setting is unavailable".into());
        }
        Ok(send_bool(workspace, selector) as u8 != 0)
    }
}

struct AutoreleasePool(Id);

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        // SAFETY: this owns the non-null NSAutoreleasePool created above. `drain`
        // consumes that pool on the same main thread and returns no value.
        unsafe {
            let send_void: unsafe extern "C" fn(Id, Sel) =
                std::mem::transmute(objc_msgSend as *const ());
            send_void(self.0, sel_registerName(c"drain".as_ptr()));
        }
    }
}

pub(super) fn wheel_scroll_lines() -> Result<u32, String> {
    Err("macOS does not expose Windows wheel-line normalization".into())
}
