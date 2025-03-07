// Copyright 2022 Google LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use core::{marker::PhantomData, ops::Deref, pin::Pin};

use std::ops::DerefMut;
#[cfg(nightly)]
use std::{marker::Unsize, ops::DispatchFromDyn, ops::Receiver};

use cxx::{memory::UniquePtrTarget, UniquePtr};

/// A C++ const reference. These are different from Rust's `&T` in that
/// these may exist even while the object is mutated elsewhere. See also
/// [`CppMutRef`] for the mutable equivalent.
///
/// The key rule is: we *never* dereference these in Rust. Therefore, any
/// UB here cannot manifest within Rust, but only across in C++, and therefore
/// they are equivalently safe to using C++ references in pure-C++ codebases.
///
/// *Important*: you might be wondering why you've never encountered this type.
/// These exist in autocxx-generated bindings only if the `unsafe_references_wrapped`
/// safety policy is given. This may become the default in future.
///
/// # Usage
///
/// These types of references are pretty useless in Rust. You can't do
/// field access. But, you can pass them back into C++! And specifically,
/// you can call methods on them (i.e. use this type as a `this`). So
/// the common case here is when C++ gives you a reference to some type,
/// then you want to call methods on that reference.
///
/// # Calling methods
///
/// As noted, one of the main reasons for this type is to call methods.
/// Currently, that depends on unstable Rust features. If you can't
/// call methods on one of these references, check you're using nightly
/// and add `#![feature(arbitrary_self_types)]` to your crate.
///
/// # Lifetimes
///
/// A `CppRef` is not associated with any Rust lifetime. Normally, for
/// ergonomics, you actually may want a lifetime associated.
/// [`CppLtRef`] gives you this.
///
/// # Field access
///
/// Field access would be achieved by adding C++ `get` and/or `set` methods.
/// It's possible that a future version of `autocxx` could generate such
/// getters and setters automatically, but they would need to be `unsafe`
/// because there is no guarantee that the referent of a `CppRef` is actually
/// what it's supposed to be, or alive. `CppRef`s may flow from C++ to Rust
/// via arbitrary means, and with sufficient uses of `get` and `set` it would
/// even be possible to create a use-after-free in pure Rust code (for instance,
/// store a [`CppPin`] in a struct field, get a `CppRef` to its referent, then
/// use a setter to reset that field of the struct.)
///
/// # Nullness
///
/// Creation of a null C++ reference is undefined behavior (because such
/// a reference can only be created by dereferencing a null pointer.)
/// However, in practice, they exist, and we need to be compatible with
/// pre-existing C++ APIs even if they do naughty things like this.
/// Therefore this `CppRef` type does allow null values. This is a bit
/// unfortunate because it means `Option<CppRef<T>>`
/// occupies more space than `CppRef<T>`.
///
/// # Dynamic dispatch
///
/// You might wonder if you can do this:
/// ```ignore
/// let CppRef<dyn Trait> = ...; // obtain some CppRef<concrete type>
/// ```
/// Dynamic dispatch works so long as you're using nightly (we require another
/// unstable feature, `dispatch_from_dyn`). But we need somewhere to store
/// the trait object, and `CppRef` isn't it -- a `CppRef` can only store a
/// simple pointer to something else. So, you need to store the trait object
/// in a `Box` or similar:
/// ```ignore
/// trait SomeTrait {
///    fn some_method(self: CppRef<Self>)
/// }
/// impl SomeTrait for ffi::Concrete {
///   fn some_method(self: CppRef<Self>) {}
/// }
/// let obj: Pin<Box<dyn SomeTrait>> = ffi::Concrete::new().within_box();
/// let obj = CppPin::from_pinned_box(obj);
/// farm_area.as_cpp_ref().some_method();
/// ```
///
/// # Implementation notes
///
/// Internally, this is represented as a raw pointer in Rust. See the note above
/// about Nullness for why we don't use [`core::ptr::NonNull`].
#[repr(transparent)]
pub struct CppRef<T: ?Sized>(*const T);

impl<T: ?Sized> CppRef<T> {
    /// Retrieve the underlying C++ pointer.
    pub fn as_ptr(&self) -> *const T {
        self.0
    }

    /// Get a regular Rust reference out of this C++ reference.
    ///
    /// # Safety
    ///
    /// Callers must guarantee that the referent is not modified by any other
    /// C++ or Rust code while the returned reference exists. Callers must
    /// also guarantee that no mutable Rust reference is created to the
    /// referent while the returned reference exists.
    ///
    /// Callers must also be sure that the C++ reference is properly
    /// aligned, not null, pointing to valid data, etc.
    pub unsafe fn as_ref(&self) -> &T {
        &*self.as_ptr()
    }

    /// Create a C++ reference from a raw pointer.
    pub fn from_ptr(ptr: *const T) -> Self {
        Self(ptr)
    }

    /// Create a mutable version of this reference, roughly equivalent
    /// to C++ `const_cast`.
    ///
    /// The opposite is to use [`AsCppRef::as_cpp_ref`] on a [`CppMutRef`]
    /// to obtain a [`CppRef`].
    ///
    /// # Safety
    ///
    /// Because we never dereference a `CppRef` in Rust, this cannot create
    /// undefined behavior _within Rust_ and is therefore not unsafe. It is
    /// however generally unwise, just as it is in C++. Use sparingly.
    pub fn const_cast(&self) -> CppMutRef<T> {
        CppMutRef(self.0 as *mut T)
    }
}

#[cfg(nightly)]
impl<T: ?Sized> Receiver for CppRef<T> {
    type Target = T;
}

impl<T: ?Sized> Clone for CppRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for CppRef<T> {}

#[cfg(nightly)]
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<CppRef<U>> for CppRef<T> {}

/// A [`CppRef`] with an associated lifetime. This can be used in place of
/// any `CppRef` due to a `Deref` implementation.
#[repr(transparent)]
pub struct CppLtRef<'a, T: ?Sized> {
    ptr: CppRef<T>,
    phantom: PhantomData<&'a T>,
}

impl<T: ?Sized> Deref for CppLtRef<'_, T> {
    type Target = CppRef<T>;
    fn deref(&self) -> &Self::Target {
        // Safety: this type is transparent and contains a CppRef<T> as
        // its only non-zero field.
        unsafe { std::mem::transmute(self) }
    }
}

impl<T: ?Sized> Clone for CppLtRef<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for CppLtRef<'_, T> {}

impl<T: ?Sized> CppLtRef<'_, T> {
    /// Extend the lifetime of the returned reference beyond normal Rust
    /// borrow checker rules.
    ///
    /// Normally, a reference can't be used beyond the lifetime of the object
    /// which gave it to you, but sometimes C++ APIs can return references
    /// to global or other longer-lived objects. In such a case you should
    /// use this method to get a longer-lived reference.
    ///
    /// # Usage
    ///
    /// When you're given a C++ reference and you know its referent is valid
    /// for a long time, use this method. Store the resulting `CppRef`
    /// somewhere in Rust with an equivalent lifetime.
    ///
    /// # Safety
    ///
    /// Because `CppRef`s are never dereferenced in Rust, misuse of this API
    /// cannot lead to undefined behavior _in Rust_ and is therefore not
    /// unsafe. Nevertheless this can lead to UB in C++, so use carefully.
    pub fn lifetime_cast(&self) -> CppRef<T> {
        CppRef(self.ptr.as_ptr())
    }

    /// Create a C++ reference from a raw pointer.
    pub fn from_ptr(ptr: *const T) -> Self {
        Self {
            ptr: CppRef::from_ptr(ptr),
            phantom: PhantomData,
        }
    }
}

/// A C++ non-const reference. These are different from Rust's `&mut T` in that
/// several C++ references can exist to the same underlying data ("aliasing")
/// and that's not permitted for regular Rust references.
///
/// See [`CppRef`] for details on safety, usage models and implementation.
///
/// You can convert this to a [`CppRef`] using the [`std::convert::Into`] trait.
#[repr(transparent)]
pub struct CppMutRef<T: ?Sized>(*mut T);

impl<T: ?Sized> CppMutRef<T> {
    /// Retrieve the underlying C++ pointer.
    pub fn as_mut_ptr(&self) -> *mut T {
        self.0
    }

    /// Get a regular Rust mutable reference out of this C++ reference.
    ///
    /// # Safety
    ///
    /// Callers must guarantee that the referent is not modified by any other
    /// C++ or Rust code while the returned reference exists. Callers must
    /// also guarantee that no other Rust reference is created to the referent
    /// while the returned reference exists.
    ///
    /// Callers must also be sure that the C++ reference is properly
    /// aligned, not null, pointing to valid data, etc.
    pub unsafe fn as_mut(&mut self) -> &mut T {
        &mut *self.as_mut_ptr()
    }

    /// Create a C++ reference from a raw pointer.
    pub fn from_ptr(ptr: *mut T) -> Self {
        Self(ptr)
    }
}

/// We implement `Deref` for `CppMutRef` so that any non-mutable
/// methods can be called on a `CppMutRef` instance.
impl<T: ?Sized> Deref for CppMutRef<T> {
    type Target = CppRef<T>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        // Safety: `CppMutRef<T>` and `CppRef<T>` have the same
        // layout.
        unsafe { std::mem::transmute(self) }
    }
}

impl<T: ?Sized> Clone for CppMutRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for CppMutRef<T> {}

impl<T> From<CppMutRef<T>> for CppRef<T> {
    fn from(mutable: CppMutRef<T>) -> Self {
        Self(mutable.0)
    }
}

#[repr(transparent)]
pub struct CppMutLtRef<'a, T: ?Sized> {
    ptr: CppMutRef<T>,
    phantom: PhantomData<&'a mut T>,
}

impl<T: ?Sized> CppMutLtRef<'_, T> {
    /// Extend the lifetime of the returned reference beyond normal Rust
    /// borrow checker rules. See [`CppLtRef::lifetime_cast`].
    pub fn lifetime_cast(&mut self) -> CppMutRef<T> {
        CppMutRef(self.ptr.as_mut_ptr())
    }

    /// Create a C++ reference from a raw pointer.
    pub fn from_ptr(ptr: *mut T) -> Self {
        Self {
            ptr: CppMutRef::from_ptr(ptr),
            phantom: PhantomData,
        }
    }
}

#[cfg(nightly)]
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<CppMutRef<U>> for CppMutRef<T> {}

/// Any type which can return a C++ reference to its contents.
pub trait AsCppRef<T: ?Sized> {
    /// Returns a reference which obeys C++ reference semantics
    fn as_cpp_ref(&self) -> CppRef<T>;
}

/// Any type which can return a C++ reference to its contents.
pub trait AsCppMutRef<T: ?Sized>: AsCppRef<T> {
    /// Returns a mutable reference which obeys C++ reference semantics
    fn as_cpp_mut_ref(&mut self) -> CppMutRef<T>;
}

impl<T: ?Sized> AsCppRef<T> for CppMutRef<T> {
    fn as_cpp_ref(&self) -> CppRef<T> {
        CppRef::from_ptr(self.0 as *const T)
    }
}

/// Workaround for the inability to use std::ptr::addr_of! on the contents
/// of a box.
#[repr(transparent)]
struct CppPinContents<T: ?Sized>(T);

impl<T: ?Sized> CppPinContents<T> {
    fn addr_of(&self) -> *const T {
        std::ptr::addr_of!(self.0)
    }
    fn addr_of_mut(&mut self) -> *mut T {
        std::ptr::addr_of_mut!(self.0)
    }
}

/// A newtype wrapper which causes the contained object to obey C++ reference
/// semantics rather than Rust reference semantics. That is, multiple aliasing
/// mutable C++ references may exist to the contents.
///
/// C++ references are permitted to alias one another, and commonly do.
/// Rust references must alias according only to the narrow rules of the
/// borrow checker.
///
/// If you need C++ to access your Rust object, first imprison it in one of these
/// objects, then use [`Self::as_cpp_ref`] to obtain C++ references to it.
/// If you need the object back for use in the Rust domain, use [`CppPin::extract`],
/// but be aware of the safety invariants that you - as a human - will need
/// to guarantee.
///
/// # Usage models
///
/// From fairly safe to fairly unsafe:
///
/// * *Configure a thing in Rust then give it to C++*. Take your Rust object,
///   set it up freely using Rust references, methods and data, then imprison
///   it in a `CppPin` and keep it around while you work with it in C++.
///   There is no possibility of _aliasing_ UB in this usage model, but you
///   still need to be careful of use-after-free bugs, just as if you were
///   to create a reference to any data in C++. The Rust borrow checker will
///   help you a little by ensuring that your `CppRef` objects don't outlive
///   the `CppPin`, but once those references pass into C++, it can't help.
/// * *Pass a thing to C++, have it operate on it synchronously, then take
///   it back*. To do this, you'd imprison your Rust object in a `CppPin`,
///   then pass mutable C++ references (using [`AsCppMutRef::as_cpp_mut_ref`])
///   into a C++ function. C++ would duly operate on the object, and thereafter
///   you could reclaim the object with `extract()`. At this point, you (as
///   a human) will need to give a guarantee that no references remain in the
///   C++ domain. If your object was just locally used by a single C++ function,
///   which has now returned, this type of local analysis may well be practical.
/// * *Share a thing between Rust and C++*. This object can vend both C++
///   references and Rust references (via `as_ref` etc.) It may be possible
///   for you to guarantee that C++ does not mutate the object while any Rust
///   reference exists. If you choose this model, you'll need to carefully
///   track exactly what happens to references and pointers on both sides,
///   and document your evidence for why you are sure this is safe.
///   Failure here is bad: Rust makes all sorts of optimization decisions based
///   upon its borrow checker guarantees, so mistakes can lead to undebuggable
///   action-at-a-distance crashes.
///
/// # See also
///
/// See also [`CppUniquePtrPin`], which is equivalent for data which is in
/// a [`cxx::UniquePtr`].
// We also keep a `CppMutRef` to the contents for the sake of our `Deref`
// implementation.
pub struct CppPin<T: ?Sized>(Box<CppPinContents<T>>, CppMutRef<T>);

impl<T: ?Sized> CppPin<T> {
    /// Imprison the Rust data within a `CppPin`. This eliminates any remaining
    /// Rust references (since we take the item by value) and this object
    /// subsequently only vends C++ style references, not Rust references,
    /// until or unless `extract` is called.
    pub fn new(item: T) -> Self
    where
        T: Sized,
    {
        let mut contents = Box::new(CppPinContents(item));
        let ptr = contents.addr_of_mut();
        Self(contents, CppMutRef(ptr))
    }

    /// Imprison the boxed Rust data within a `CppPin`. This eliminates any remaining
    /// Rust references (since we take the item by value) and this object
    /// subsequently only vends C++ style references, not Rust references,
    /// until or unless `extract` is called.
    ///
    /// If the item is already in a `Box`, this is slightly more efficient than
    /// `new` because it will avoid moving/reallocating it.
    pub fn from_box(item: Box<T>) -> Self {
        // Safety: CppPinContents<T> is #[repr(transparent)] so
        // this transmute from
        //   Box<T>
        // to
        //   Box<CppPinContents<T>>
        // is safe.
        let mut contents = unsafe { std::mem::transmute::<Box<T>, Box<CppPinContents<T>>>(item) };
        let ptr = contents.addr_of_mut();
        Self(contents, CppMutRef(ptr))
    }

    // Imprison the boxed Rust data within a `CppPin`.  This eliminates any remaining
    /// Rust references (since we take the item by value) and this object
    /// subsequently only vends C++ style references, not Rust references,
    /// until or unless `extract` is called.
    ///
    /// If the item is already in a `Box`, this is slightly more efficient than
    /// `new` because it will avoid moving/reallocating it.
    pub fn from_pinned_box(item: Pin<Box<T>>) -> Self {
        // Safety: it's OK to un-pin the Box because we'll be putting it
        // into a CppPin which upholds the same pinned-ness contract.
        Self::from_box(unsafe { Pin::into_inner_unchecked(item) })
    }

    /// Get an immutable pointer to the underlying object.
    pub fn as_ptr(&self) -> *const T {
        self.0.addr_of()
    }

    /// Get a mutable pointer to the underlying object.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.0.addr_of_mut()
    }

    /// Get a normal Rust reference to the underlying object. This is unsafe.
    ///
    /// # Safety
    ///
    /// You must guarantee that C++ will not mutate the object while the
    /// reference exists.
    pub unsafe fn as_ref(&self) -> &T {
        &*self.as_ptr()
    }

    /// Get a normal Rust mutable reference to the underlying object. This is unsafe.
    ///
    /// # Safety
    ///
    /// You must guarantee that C++ will not mutate the object while the
    /// reference exists.
    pub unsafe fn as_mut(&mut self) -> &mut T {
        &mut *self.as_mut_ptr()
    }

    /// Extract the object from within its prison, for re-use again within
    /// the domain of normal Rust references.
    ///
    /// This returns a `Box<T>`: if you want the underlying `T` you can extract
    /// it using `*`.
    ///
    /// # Safety
    ///
    /// Callers promise that no remaining C++ references exist either
    /// in the form of Rust [`CppRef`]/[`CppMutRef`] or any remaining pointers/
    /// references within C++.
    pub unsafe fn extract(self) -> Box<T> {
        // Safety: CppPinContents<T> is #[repr(transparent)] so
        // this transmute from
        //   Box<CppPinContents<T>>
        // to
        //   Box<T>
        // is safe.
        std::mem::transmute(self.0)
    }
}

impl<T: ?Sized> AsCppRef<T> for CppPin<T> {
    fn as_cpp_ref(&self) -> CppRef<T> {
        CppRef::from_ptr(self.as_ptr())
    }
}

impl<T: ?Sized> AsCppMutRef<T> for CppPin<T> {
    fn as_cpp_mut_ref(&mut self) -> CppMutRef<T> {
        CppMutRef::from_ptr(self.as_mut_ptr())
    }
}

impl<T: ?Sized> Deref for CppPin<T> {
    type Target = CppMutRef<T>;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl<T: ?Sized> DerefMut for CppPin<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.1
    }
}

/// Any newtype wrapper which causes the contained [`UniquePtr`] target to obey C++ reference
/// semantics rather than Rust reference semantics. That is, multiple aliasing
/// mutable C++ references may exist to the contents.
///
/// C++ references are permitted to alias one another, and commonly do.
/// Rust references must alias according only to the narrow rules of the
/// borrow checker.
pub struct CppUniquePtrPin<T: UniquePtrTarget>(UniquePtr<T>, CppMutRef<T>);

impl<T: UniquePtrTarget> CppUniquePtrPin<T> {
    /// Imprison the type within a `CppPin`. This eliminates any remaining
    /// Rust references (since we take the item by value) and this object
    /// subsequently only vends C++ style references, not Rust references.
    pub fn new(item: UniquePtr<T>) -> Self {
        let ptr = item.as_mut_ptr();
        Self(item, CppMutRef::from_ptr(ptr))
    }

    /// Get an immutable pointer to the underlying object.
    pub fn as_ptr(&self) -> *const T {
        // TODO - avoid brief reference here
        self.0
            .as_ref()
            .expect("UniquePtr was null; we can't make a C++ reference")
    }
}

impl<T: UniquePtrTarget> AsCppRef<T> for CppUniquePtrPin<T> {
    fn as_cpp_ref(&self) -> CppRef<T> {
        CppRef::from_ptr(self.as_ptr())
    }
}

impl<T: UniquePtrTarget> AsCppMutRef<T> for CppUniquePtrPin<T> {
    fn as_cpp_mut_ref(&mut self) -> CppMutRef<T> {
        self.1
    }
}

impl<T: UniquePtrTarget> Deref for CppUniquePtrPin<T> {
    type Target = CppMutRef<T>;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

// It would be very nice to be able to impl Deref for UniquePtr
impl<T: UniquePtrTarget> AsCppRef<T> for cxx::UniquePtr<T> {
    fn as_cpp_ref(&self) -> CppRef<T> {
        CppRef::from_ptr(self.as_ptr())
    }
}

#[cfg(all(feature = "arbitrary_self_types", test))]
mod tests {
    use super::*;

    struct CppOuter {
        _a: u32,
        inner: CppInner,
        global: *const CppInner,
    }

    impl CppOuter {
        fn get_inner_ref<'a>(self: &CppRef<'a, CppOuter>) -> CppRef<'a, CppInner> {
            // Safety: emulating C++ code for test purposes. This is safe
            // because we know the data isn't modified during the lifetime of
            // the returned reference.
            let self_rust_ref = unsafe { self.as_ref() };
            CppRef::from_ptr(std::ptr::addr_of!(self_rust_ref.inner))
        }
        fn get_global_ref<'a>(self: &CppRef<'a, CppOuter>) -> CppRef<'a, CppInner> {
            // Safety: emulating C++ code for test purposes. This is safe
            // because we know the data isn't modified during the lifetime of
            // the returned reference.
            let self_rust_ref = unsafe { self.as_ref() };
            CppRef::from_ptr(self_rust_ref.global)
        }
    }

    struct CppInner {
        b: u32,
    }

    impl CppInner {
        fn value_is(self: &CppRef<Self>) -> u32 {
            // Safety: emulating C++ code for test purposes. This is safe
            // because we know the data isn't modified during the lifetime of
            // the returned reference.
            let self_rust_ref = unsafe { self.as_ref() };
            self_rust_ref.b
        }
    }

    #[test]
    fn cpp_objects() {
        let mut global = CppInner { b: 7 };
        let global_ref_lifetime_phantom;
        {
            let outer = CppOuter {
                _a: 12,
                inner: CppInner { b: 3 },
                global: &mut global,
            };
            let outer = CppPin::new(outer);
            let inner_ref = outer.as_cpp_ref().get_inner_ref();
            assert_eq!(inner_ref.value_is(), 3);
            global_ref_lifetime_phantom = Some(outer.as_cpp_ref().get_global_ref().lifetime_cast());
        }
        let global_ref = global_ref_lifetime_phantom.unwrap();
        let global_ref = global_ref.as_cpp_ref();
        assert_eq!(global_ref.value_is(), 7);
    }

    #[test]
    fn cpp_pin() {
        let a = RustThing { _a: 4 };
        let a = CppPin::new(a);
        let _ = a.as_cpp_ref();
        let _ = a.as_cpp_ref();
    }
}
