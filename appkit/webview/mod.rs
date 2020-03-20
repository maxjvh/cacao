//! Implements a WebView, which wraps a number of different classes/delegates/controllers into one
//! useful interface. This encompasses...
//!
//! - `WKWebView`
//! - `WKUIDelegate`
//! - `WKScriptMessageHandler`
//! - `NSViewController`

use std::rc::Rc;
use std::cell::RefCell;

use objc_id::ShareId;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use crate::foundation::{id, nil, YES, NO, NSString};
use crate::constants::WEBVIEW_CONTROLLER_PTR;
use crate::webview::controller::register_controller_class;

pub mod actions;

pub(crate) mod controller;
//pub(crate) mod process_pool;

pub mod traits;
pub use traits::WebViewController;

pub mod config;
pub use config::WebViewConfig;

/// A `View` wraps two different controllers - one on the Objective-C/Cocoa side, which forwards
/// calls into your supplied `ViewController` trait object. This involves heap allocation, but all
/// of Cocoa is essentially Heap'd, so... well, enjoy.
#[derive(Clone)]
pub struct WebView<T> {
    internal_callback_ptr: *const RefCell<T>,
    pub objc_controller: WebViewHandle,
    pub controller: Rc<RefCell<T>>
}

impl<T> WebView<T> where T: WebViewController + 'static {
    /// Allocates and configures a `ViewController` in the Objective-C/Cocoa runtime that maps over
    /// to your supplied view controller.
    pub fn new(controller: T) -> Self {
        let config = controller.config();
        let controller = Rc::new(RefCell::new(controller));
        
        let internal_callback_ptr = {
            let cloned = Rc::clone(&controller);
            Rc::into_raw(cloned)
        };

        let handle = WebViewHandle::new(unsafe {
            let view_controller: id = msg_send![register_controller_class::<T>(), new];
            (&mut *view_controller).set_ivar(WEBVIEW_CONTROLLER_PTR, internal_callback_ptr as usize);
            
            // WKWebView isn't really great to subclass, so we don't bother here unlike other
            // widgets in this framework. Just set and forget.
            let frame: CGRect = Rect::zero().into();
            let alloc: id = msg_send![class!(WKWebView), alloc];
            let view: id = msg_send![alloc, initWithFrame:frame configuration:&*config.0];
            let _: () = msg_send![&*view_controller, setView:view];
            
            ShareId::from_ptr(view_controller)
        });

        {
            let mut vc = controller.borrow_mut();
            (*vc).did_load(handle.clone());
        }

        WebView {
            internal_callback_ptr: internal_callback_ptr,
            objc_controller: handle,
            controller: controller
        }
    }

    pub fn set_background_color(&self, color: Color) {
        self.objc_controller.set_background_color(color);
    }

    pub fn register_for_dragged_types(&self, types: &[PasteboardType]) {
        self.objc_controller.register_for_dragged_types(types);
    }

    pub fn top(&self) -> &LayoutAnchorY {
        &self.objc_controller.top
    }

    pub fn leading(&self) -> &LayoutAnchorX {
        &self.objc_controller.leading
    }

    pub fn trailing(&self) -> &LayoutAnchorX {
        &self.objc_controller.trailing
    }

    pub fn bottom(&self) -> &LayoutAnchorY {
        &self.objc_controller.bottom
    }

    pub fn width(&self) -> &LayoutAnchorDimension {
        &self.objc_controller.width
    }

    pub fn height(&self) -> &LayoutAnchorDimension {
        &self.objc_controller.height
    }
}

impl<T> Layout for View<T> {
    /// Returns the Objective-C object used for handling the view heirarchy.
    fn get_backing_node(&self) -> Option<ShareId<Object>> {
        self.objc_controller.objc.clone()
    }

    fn add_subview<V: Layout>(&self, subview: &V) {
        self.objc_controller.add_subview(subview);
    }
}

impl<T> std::fmt::Debug for View<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "View ({:p})", self)
    }
}

impl<T> Drop for View<T> {
    /// A bit of extra cleanup for delegate callback pointers.
    fn drop(&mut self) {
        unsafe {
            let _ = Rc::from_raw(self.internal_callback_ptr);
        }
    }
}