use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, RwLock};

use lazy_static::lazy_static;

use objc::{class, msg_send, sel, sel_impl};
use objc::declare::ClassDecl;
use objc::runtime::{objc_getClass, Class, Object};

lazy_static! {
    static ref CLASSES: ClassMap = ClassMap::new();
}

/// A temporary method for testing; this will get cleaned up if it's worth bringing in permanently.
///
/// (and probably not repeatedly queried...)
///
/// This accounts for code not running in a standard bundle, and returns `None` if the bundle
/// identifier is nil.
fn get_bundle_id() -> Option<String> {
    let identifier: *mut Object = unsafe {
        let bundle: *mut Object = msg_send![class!(NSBundle), mainBundle];
        msg_send![bundle, bundleIdentifier]
    };

    if identifier == crate::foundation::nil {
        return None;
    }
    
    let identifier = crate::foundation::NSString::retain(identifier).to_string()
        .replace(".", "_")
        .replace("-", "_");

    Some(identifier)
}

/// Represents an entry in a `ClassMap`. We store an optional superclass_name for debugging
/// purposes; it's an `Option` to make the logic of loading a class type where we don't need to 
/// care about the superclass type simpler.
#[derive(Debug)]
struct ClassEntry {
    pub superclass_name: Option<&'static str>,
    pub ptr: usize
}

/// Represents a key in a `ClassMap`.
type ClassKey = (&'static str, Option<&'static str>);

/// A ClassMap is a general cache for our Objective-C class lookup and registration. Rather than
/// constantly calling into the runtime, we store pointers to Class types here after first lookup
/// and/or creation.
///
/// There may be a way to do this without using HashMaps and avoiding the heap, but working and
/// usable beats ideal for now. Open to suggestions.
#[derive(Debug)]
pub(crate) struct ClassMap(RwLock<HashMap<ClassKey, ClassEntry>>);

impl ClassMap {
    /// Returns a new ClassMap.
    pub fn new() -> Self {
        ClassMap(RwLock::new(HashMap::new()))
    }

    /// Attempts to load a previously registered class.
    ///
    /// This checks our internal map first, and then calls out to the Objective-C runtime to ensure
    /// we're not missing anything.
    pub fn load(&self, class_name: &'static str, superclass_name: Option<&'static str>) -> Option<*const Class> {
        {
            let reader = self.0.read().unwrap();
            if let Some(entry) = (*reader).get(&(class_name, superclass_name)) {
                let ptr = &entry.ptr;
                return Some(*ptr as *const Class);
            }
        }

        // If we don't have an entry for the class_name in our internal map, we should still check
        // if we can load it from the Objective-C runtime directly. The reason we need to do this
        // is that there's a use-case where someone might have multiple bundles attempting to
        // use or register the same subclass; Rust doesn't see the same pointers unlike the Objective-C
        // runtime, and we can wind up in a situation where we're attempting to register a Class
        // that already exists but we can't see.
        let objc_class_name = CString::new(class_name).unwrap();
        let class = unsafe { objc_getClass(objc_class_name.as_ptr() as *const _) };

        // This should not happen for our use-cases, but it's conceivable that this could actually
        // be expected, so just return None and let the caller panic if so desired.
        if class.is_null() {
            return None;
        }

        // If we got here, then this class exists in the Objective-C runtime but is not known to
        // us. For consistency's sake, we'll add this to our store and return that.
        {
            let mut writer = self.0.write().unwrap();
            writer.insert((class_name, superclass_name), ClassEntry {
                superclass_name,
                ptr: class as usize
            });
        }

        Some(class)
    }

    /// Store a newly created subclass type.
    pub fn store(&self, class_name: &'static str, superclass_name: Option<&'static str>, class: *const Class) {
        let mut writer = self.0.write().unwrap();

        writer.insert((class_name, superclass_name), ClassEntry {
            superclass_name,
            ptr: class as usize
        });
    }
}

/// Attempts to load a subclass, given a `superclass_name` and subclass_name. If
/// the subclass cannot be loaded, it's dynamically created and injected into
/// the runtime, and then returned. The returned value can be used for allocating new instances of
/// this class in the Objective-C runtime.
///
/// The `config` block can be used to customize the Class declaration before it's registered with
/// the runtime. This is useful for adding method handlers and ivar storage.
///
/// If the superclass cannot be loaded, this will panic. If the subclass cannot be
/// created, this will panic. In general, this is expected to work, and if it doesn't,
/// the entire framework will not really work.
///
/// There's definitely room to optimize here, but it works for now.
#[inline(always)]
pub fn load_or_register_class<F>(
    superclass_name: &'static str,
    subclass_name: &'static str,
    config: F
) -> *const Class
where
    F: Fn(&mut ClassDecl) + 'static
{
    if let Some(subclass) = CLASSES.load(subclass_name, Some(superclass_name)) {
        return subclass;
    }

    // If we can't find the class anywhere, then we'll attempt to load the superclass and register
    // our new class type.
    if let Some(superclass) = CLASSES.load(superclass_name, None) {
        let objc_subclass_name = match get_bundle_id() {
            Some(bundle_id) => format!("{}_{}_{}", subclass_name, superclass_name, bundle_id),
            None => format!("{}_{}", subclass_name, superclass_name)
        };

        match ClassDecl::new(&objc_subclass_name, unsafe { &*superclass }) {
            Some(mut decl) => {
                config(&mut decl);

                let class = decl.register();
                CLASSES.store(subclass_name, Some(superclass_name), class);
                return class;
            },

            None => {
                panic!(
                    "Subclass of type {}_{} could not be allocated.",
                    subclass_name, superclass_name
                );
            }
        }
    }

    panic!(
        "Attempted to create subclass for {}, but unable to load superclass of type {}.",
        subclass_name, superclass_name
    );
}
