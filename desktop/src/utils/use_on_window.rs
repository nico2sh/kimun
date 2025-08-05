use dioxus::{logger::tracing::error, prelude::*};
use std::{cell::RefCell, ops::Deref, rc::Rc};
use wasm_bindgen::{
    closure::Closure,
    convert::{FromWasmAbi, RefFromWasmAbi},
    JsCast,
};
use web_sys::EventTarget;

pub(crate) fn use_on_window<EventKind>(
    event_name: &'static str,
    callback: impl FnMut(EventKind) + 'static,
) where
    EventKind: Sized + RefFromWasmAbi + FromWasmAbi + Clone + 'static,
{
    let window = web_sys::window().expect("no window available");
    use_on_event(&window, event_name, callback);
}

/// Hook to listen to a specific event on a specific element.
pub(crate) fn use_on_event<EventKind, T>(
    target_element: &T,
    event_name: &'static str,
    mut callback: impl FnMut(EventKind) + 'static,
) where
    EventKind: Sized + RefFromWasmAbi + FromWasmAbi + Clone + 'static,
    T: Clone + Deref<Target = EventTarget> + std::fmt::Debug + 'static,
{
    let hook = || {
        EventListenerHandle::new(target_element.clone(), event_name, move |kind| {
            callback(kind)
        })
    };

    let cleanup = |f: EventListenerHandle| {
        f.cleanup();
    };

    use_hook_with_cleanup(hook, cleanup);
}

#[derive(Clone)]
pub(crate) struct EventListenerHandle {
    cleanup: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
}

impl EventListenerHandle {
    pub(crate) fn new<EventKind, T>(
        target_element: T,
        event_name: &'static str,
        mut callback: impl FnMut(EventKind) + 'static,
    ) -> Self
    where
        EventKind: Sized + RefFromWasmAbi + FromWasmAbi + Clone + 'static,
        T: Clone + Deref<Target = EventTarget> + std::fmt::Debug + 'static,
    {
        let closure = Closure::wrap(Box::new(move |event: EventKind| {
            callback(event);
        }) as Box<dyn FnMut(_)>);

        if let Err(e) = target_element
            .add_event_listener_with_callback(&event_name, closure.as_ref().unchecked_ref())
        {
            error!("failed to add event listener: {e:?}");
        }

        let cleanup = Rc::new(RefCell::new(Some(Box::new(move || {
            if let Err(e) = target_element
                .remove_event_listener_with_callback(&event_name, closure.as_ref().unchecked_ref())
            {
                error!("failed to remove event listener: {e:?}");
            }
        }) as Box<dyn FnOnce()>)));
        Self { cleanup }
    }

    pub(crate) fn cleanup(&self) {
        let cleanup = self.cleanup.borrow_mut().take();
        if let Some(cleanup) = cleanup {
            cleanup();
        }
    }
}

impl Drop for EventListenerHandle {
    fn drop(&mut self) {
        // Only cleanup if this is the last reference.
        if Rc::strong_count(&self.cleanup) == 1 {
            self.cleanup();
        }
    }
}
