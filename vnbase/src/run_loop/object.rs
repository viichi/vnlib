use std::marker::PhantomData;
use std::mem;
use std::sync::atomic;
use std::sync::atomic::AtomicUsize;
use std::isize;
use std::process::abort;
use std::ptr;

pub trait Object {
    fn set_next(&mut self, obj: Option<*mut Object>);
    fn get_next(&self) -> Option<*mut Object>;
    fn set_prev(&mut self, obj: Option<*mut Object>);
    fn get_prev(&self) -> Option<*mut Object>;
}

struct ObjectNode<T: 'static> {
    handle: *mut ObjH<T>,
    next: Option<*mut Object>,
    prev: Option<*mut Object>,
    obj: T,
}

impl<T> Object for ObjectNode<T> {
    fn set_next(&mut self, obj: Option<*mut Object>) {
        self.next = obj;
    }

    fn get_next(&self) -> Option<*mut Object> {
        self.next
    }

    fn set_prev(&mut self, obj: Option<*mut Object>) {
        self.prev = obj;
    }

    fn get_prev(&self) -> Option<*mut Object> {
        self.prev
    }
}

pub struct ObjectList {
    head: Option<*mut Object>,
    phantom: PhantomData<Box<Object>>,
}

impl Drop for ObjectList {
    fn drop(&mut self) {
        unsafe {
            let mut head = self.head;
            while let Some(node) = head {
                let node = Box::from_raw(node);
                head = node.get_next();
            }
        }
    }
}

impl ObjectList {
    pub fn new() -> Self {
        ObjectList {
            head: None,
            phantom: PhantomData,
        }
    }

    pub fn create<T>(&mut self, obj: T) -> ObjectHandle<T>
        where T: 'static {
        unsafe {
            let node = Box::new(ObjectNode {
                handle: mem::uninitialized(),
                next: self.head,
                prev: None,
                obj: obj,
            });

            let node = Box::into_raw(node);

            (*node).handle = Box::into_raw(Box::new(ObjH {
                ptr: node,
                strong: AtomicUsize::new(1),
                weak: AtomicUsize::new(1),
            }));

            if let Some(ref mut head) = self.head {
                (**head).set_prev(Some(node));
                *head = node;
            }

            ObjectHandle {
                core: super::clone_handle(),
                handle: (*node).handle,
                phantom: PhantomData,
            }
        }
    }

    pub unsafe fn remove(&mut self, node: *mut Object) {
        let node = Box::from_raw(node);
        let next = node.get_next();
        let prev = node.get_prev();
        if let Some(prev) = prev {
            (*prev).set_next(next);
        }
        else {
            self.head = next;
        }
        if let Some(next) = next {
            (*next).set_prev(prev);
        }
    }
}

/// 循环内对象句柄，部分特征和std::sync::Arc类似
/// 
/// # Examples
/// ```
/// use vnbase::run_loop;
/// use std::thread;
/// 
/// struct MyObject ();
/// impl MyObject {
///     fn say_hello(&self) {
///         println!("Hello");
///     }
/// }
/// 
/// let obj = run_loop::new_object(MyObject ());
/// 
/// let handle = run_loop::clone_handle();
/// let th = thread::spawn(move || {
///     obj.post(MyObject::say_hello);
///     handle.post(run_loop::stop);
/// });
/// 
/// run_loop::run();
/// th.join().unwrap();
/// ```
pub struct ObjectHandle<T: 'static> {
    core: super::Handle,
    handle: *mut ObjH<T>,
    phantom: PhantomData<T>,
}

unsafe impl<T> Send for ObjectHandle<T> {}
unsafe impl<T> Sync for ObjectHandle<T> {}

impl<T> Clone for ObjectHandle<T> {
    fn clone(&self) -> Self {
        
        unsafe { ObjH::inc_strong(self.handle); }

        ObjectHandle {
            core: self.core.clone(),
            handle: self.handle,
            phantom: PhantomData,
        }
    }
}

impl<T> Drop for ObjectHandle<T> {
    fn drop(&mut self) {
        if let Some(ptr) = unsafe { ObjH::dec_strong(self.handle) } {
            if super::is_own_handle(&self.core) {
                unsafe { super::drop_object(ptr.0); }
            }
            else {
                self.core.post(move || unsafe { super::drop_object(ptr.0) });
            }
        }
    }
}

impl<T> ObjectHandle<T> {
    pub fn post<F>(&self, msg: F) where F: FnOnce(&T) + 'static + Send {
        unsafe { ObjH::inc_strong(self.handle); }
        let ptr = ObjectNodePtr (unsafe { (*self.handle).ptr });
        self.core.post(move || {
            let obj = unsafe { &(*ptr.0) };
            msg(&obj.obj);
            unsafe { 
                if let Some(_) = ObjH::dec_strong(obj.handle) {
                    super::drop_object(ptr.0);
                }
            }
        })
    }

    pub fn get_ref(&self) -> Option<&T> {
        if super::is_own_handle(&self.core) {
            Some(unsafe { &(*(*self.handle).ptr).obj })
        }
        else {
            None
        }
    }

    pub fn downgrade(&self) -> ObjectWeak<T> {
        let mut n = unsafe { (*self.handle).weak.load(atomic::Ordering::Relaxed) };
        loop {
            if n == MAX_REFCOUNT {
                n = unsafe { (*self.handle).weak.load(atomic::Ordering::Relaxed) };
                continue;
            }

            if let Err(old) = unsafe { (*self.handle).weak.compare_exchange_weak(n, n + 1, atomic::Ordering::Acquire, atomic::Ordering::Relaxed) } {
                n = old;
            }
            else {
                return ObjectWeak {
                    core: self.core.clone(),
                    handle: self.handle,
                }
            }
        }
    }
}

const MAX_REFCOUNT: usize = isize::MAX as usize;

struct ObjH<T: 'static> {
    ptr: *mut ObjectNode<T>,
    strong: AtomicUsize,
    weak: AtomicUsize,
}

impl<T> ObjH<T> {
    unsafe fn inc_strong(ptr: *mut ObjH<T>) {
        if (*ptr).strong.fetch_add(1, atomic::Ordering::Relaxed) > MAX_REFCOUNT {
            abort();
        }
    }

    unsafe fn dec_strong(ptr: *mut ObjH<T>) -> Option<ObjectNodePtr<T>> {
        if (*ptr).strong.fetch_sub(1, atomic::Ordering::Release) != 1 {
            return None;
        }

        atomic::fence(atomic::Ordering::Acquire);

        let node_ptr = ObjectNodePtr((*ptr).ptr);

        if (*ptr).weak.fetch_sub(1, atomic::Ordering::Release) == 1 {
            atomic::fence(atomic::Ordering::Acquire);
            Box::from_raw(ptr);
        }

        Some(node_ptr)
    }

    unsafe fn inc_weak(ptr: *mut ObjH<T>) {
        if (*ptr).weak.fetch_add(1, atomic::Ordering::Relaxed) > MAX_REFCOUNT {
            abort();
        }
    }

    unsafe fn dec_weak(ptr: *mut ObjH<T>) {
        if (*ptr).weak.fetch_sub(1, atomic::Ordering::Release) == 1 {
            atomic::fence(atomic::Ordering::Acquire);
            Box::from_raw(ptr);
        }
    }
}

struct ObjectNodePtr<T: 'static> (*mut ObjectNode<T>);

unsafe impl<T> Send for ObjectNodePtr<T> {}

/// 循环内对象句柄的弱引用，部分特征和std::sync::Weak类似
/// 
/// # Examples
/// ```
/// use vnbase::run_loop;
/// use std::thread;
/// 
/// struct MyObject ();
/// 
/// let obj = run_loop::new_object(MyObject ());
/// let obj_weak = obj.downgrade();
/// drop(obj);
/// 
/// let handle = run_loop::clone_handle();
/// let th = thread::spawn(move || {
///     if let Some(_) = obj_weak.upgrade() {
///         unreachable!();
///     }
///     handle.post(run_loop::stop);
/// });
/// 
/// run_loop::run();
/// th.join().unwrap();
/// ```
pub struct ObjectWeak<T: 'static> {
    core: super::Handle,
    handle: *mut ObjH<T>,
}

unsafe impl<T> Send for ObjectWeak<T> {}
unsafe impl<T> Sync for ObjectWeak<T> {}

impl<T> Clone for ObjectWeak<T> {
    fn clone(&self) -> Self {
        unsafe { ObjH::inc_weak(self.handle); }
        ObjectWeak {
            core: self.core.clone(),
            handle: self.handle,
        }
    }
}

impl<T> Drop for ObjectWeak<T> {
    fn drop(&mut self) {
        unsafe { ObjH::dec_weak(self.handle); }
    }
}

impl<T> ObjectWeak<T> {
    pub fn new() -> Self {
        ObjectWeak {
            core: super::clone_handle(),
            handle: Box::into_raw(Box::new(ObjH {
                ptr: ptr::null_mut(),
                strong: AtomicUsize::new(0),
                weak: AtomicUsize::new(1),
            })),
        }
    }

    pub fn upgrade(&self) -> Option<ObjectHandle<T>> {
        let mut n = unsafe { (*self.handle).strong.load(atomic::Ordering::Relaxed) };
        loop {
            if n == 0 {
                return None;
            }

            if n > MAX_REFCOUNT {
                abort();
            }

            if let Err(old) = unsafe { (*self.handle).strong.compare_exchange_weak(n, n + 1, atomic::Ordering::Relaxed, atomic::Ordering::Relaxed) } {
                n = old;
            }
            else {
                return Some(ObjectHandle {
                    core: self.core.clone(),
                    handle: self.handle,
                    phantom: PhantomData,
                })
            }
        }
    }
}