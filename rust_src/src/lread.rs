//! Lisp parsing and input streams.

use libc;
use std::ffi::CString;

use remacs_macros::lisp_fn;
use remacs_sys;
use remacs_sys::staticpro;
use remacs_sys::EmacsInt;
use remacs_sys::{build_string, read_internal_start, symbol_redirect};
use remacs_sys::{globals, Qnil, Qread_char};

use data::{Lisp_Boolfwd, Lisp_Buffer_Objfwd, Lisp_Fwd, Lisp_Fwd_Bool, Lisp_Fwd_Buffer_Obj,
           Lisp_Fwd_Int, Lisp_Fwd_Kboard_Obj, Lisp_Fwd_Obj, Lisp_Intfwd, Lisp_Kboard_Objfwd,
           Lisp_Objfwd};
use field_offset::FieldOffset;
use lisp::{defsubr, LispObject};
use obarray::{intern, intern_c_string_1};

// Define an "integer variable"; a symbol whose value is forwarded to a
// C variable of type EMACS_INT.  Sample call (with "xx" to fool make-docfile):
// DEFxxVAR_INT ("emacs-priority", &emacs_priority, "Documentation");
#[no_mangle]
pub unsafe extern "C" fn defvar_int(
    i_fwd: *mut Lisp_Intfwd,
    namestring: *const libc::c_schar,
    address: *mut EmacsInt,
) {
    (*i_fwd).ty = Lisp_Fwd_Int;
    (*i_fwd).intvar = address;
    let sym = intern_c_string_1(namestring, libc::strlen(namestring) as libc::ptrdiff_t)
        .as_symbol_or_error();
    sym.set_declared_special(true);
    sym.set_redirect(symbol_redirect::SYMBOL_FORWARDED);
    sym.set_fwd(i_fwd as *mut Lisp_Fwd);
}

/* Similar but define a variable whose value is t if address contains 1,
   nil if address contains 0.  */
#[no_mangle]
pub unsafe extern "C" fn defvar_bool(
    b_fwd: *mut Lisp_Boolfwd,
    namestring: *const libc::c_schar,
    address: *mut bool,
) {
    (*b_fwd).ty = Lisp_Fwd_Bool;
    (*b_fwd).boolvar = address;
    let sym = intern_c_string_1(namestring, libc::strlen(namestring) as libc::ptrdiff_t)
        .as_symbol_or_error();
    sym.set_declared_special(true);
    sym.set_redirect(symbol_redirect::SYMBOL_FORWARDED);
    sym.set_fwd(b_fwd as *mut Lisp_Fwd);
}

/// Similar but define a variable whose value is the Lisp Object stored
/// at address.  Two versions: with and without gc-marking of the C
/// variable.  The nopro version is used when that variable will be
/// gc-marked for some other reason, since marking the same slot twice
/// can cause trouble with strings.
#[no_mangle]
pub unsafe extern "C" fn defvar_lisp_nopro(
    o_fwd: *mut Lisp_Objfwd,
    namestring: *const libc::c_schar,
    address: *mut LispObject,
) {
    (*o_fwd).ty = Lisp_Fwd_Obj;
    (*o_fwd).objvar = address;
    let sym = intern_c_string_1(namestring, libc::strlen(namestring) as libc::ptrdiff_t)
        .as_symbol_or_error();
    sym.set_declared_special(true);
    sym.set_redirect(symbol_redirect::SYMBOL_FORWARDED);
    sym.set_fwd(o_fwd as *mut Lisp_Fwd);
}

#[no_mangle]
pub unsafe extern "C" fn defvar_lisp(
    o_fwd: *mut Lisp_Objfwd,
    namestring: *const libc::c_schar,
    address: *mut LispObject,
) {
    defvar_lisp_nopro(o_fwd, namestring, address);
    staticpro(address);
}

/// Similar but define a variable whose value is the Lisp Object stored
/// at a particular offset in the current kboard object.
#[no_mangle]
pub unsafe extern "C" fn defvar_kboard(
    ko_fwd: *mut Lisp_Kboard_Objfwd,
    namestring: *const libc::c_schar,
    offset: i32,
) {
    defvar_kboard_offset(
        ko_fwd,
        namestring,
        FieldOffset::<remacs_sys::kboard, LispObject>::new_from_offset(offset as usize),
    )
}

pub unsafe fn defvar_kboard_offset(
    ko_fwd: *mut Lisp_Kboard_Objfwd,
    namestring: *const libc::c_schar,
    offset: FieldOffset<remacs_sys::kboard, LispObject>,
) {
    (*ko_fwd).ty = Lisp_Fwd_Kboard_Obj;
    (*ko_fwd).offset = offset;
    let sym = intern_c_string_1(namestring, libc::strlen(namestring) as libc::ptrdiff_t)
        .as_symbol_or_error();
    sym.set_declared_special(true);
    sym.set_redirect(symbol_redirect::SYMBOL_FORWARDED);
    sym.set_fwd(ko_fwd as *mut Lisp_Fwd);
}

#[no_mangle]
pub unsafe extern "C" fn defvar_per_buffer(
    bo_fwd: *mut Lisp_Buffer_Objfwd,
    namestring: *const libc::c_schar,
    offset: FieldOffset<remacs_sys::Lisp_Buffer, LispObject>,
    predicate: LispObject,
) {
    defvar_per_buffer_offset(bo_fwd, namestring, offset, predicate);
}

pub unsafe fn defvar_per_buffer_offset(
    bo_fwd: *mut Lisp_Buffer_Objfwd,
    namestring: *const libc::c_schar,
    offset: FieldOffset<remacs_sys::Lisp_Buffer, LispObject>,
    predicate: LispObject,
) {
    (*bo_fwd).ty = Lisp_Fwd_Buffer_Obj;
    (*bo_fwd).offset = offset;
    (*bo_fwd).predicate = predicate;
    let sym = intern_c_string_1(namestring, libc::strlen(namestring) as libc::ptrdiff_t)
        .as_symbol_or_error();
    sym.set_declared_special(true);
    sym.set_redirect(symbol_redirect::SYMBOL_FORWARDED);
    sym.set_fwd(bo_fwd as *mut Lisp_Fwd);
    let local = offset.apply_mut(&mut remacs_sys::buffer_local_symbols);
    *local = sym.as_lisp_obj();
    let flags = offset.apply(&remacs_sys::buffer_local_flags);
    if flags.is_nil() {
        /* Did a DEFVAR_PER_BUFFER without initializing the corresponding
           slot of buffer_local_flags.  */
        remacs_sys::emacs_abort();
    }
}

/// Read one Lisp expression as text from STREAM, return as Lisp object.
/// If STREAM is nil, use the value of `standard-input' (which see).
/// STREAM or the value of `standard-input' may be:
///  a buffer (read from point and advance it)
///  a marker (read from where it points and advance it)
///  a function (call it with no arguments for each character,
///      call it with a char as argument to push a char back)
///  a string (takes text from string, starting at the beginning)
///  t (read text line using minibuffer and use it, or read from
///     standard input in batch mode).
#[lisp_fn(min = "0")]
pub fn read(stream: LispObject) -> LispObject {
    // This function ends with a call either to read_internal_start or
    // read-minibuffer.
    //
    // read_internal_start will be called in two circumstances:
    //   1) stream is something other than t, nil, or 'read-char;
    //   2) stream is nil and standard-input is something other than t
    //      or 'read-char.
    // In all other cases, read-minibuffer will be called.

    let input = if stream.is_not_nil() {
        stream
    } else {
        unsafe { globals.Vstandard_input }
    };

    if input.is_t() || input.eq(Qread_char) {
        let cs = CString::new("Lisp expression: ").unwrap();
        call!(intern("read-minibuffer"), unsafe {
            build_string(cs.as_ptr())
        })
    } else {
        unsafe { read_internal_start(input, Qnil, Qnil) }
    }
}

include!(concat!(env!("OUT_DIR"), "/lread_exports.rs"));
