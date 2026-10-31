//! What the GUI state needs from the outside world besides [`clp2cfg_core::apply::TargetIo`].

/// The OS clipboard (injectable for tests).
pub trait ClipboardSource {
    /// The clipboard's text, `None` when it is empty or not text.
    fn text(&self) -> Option<String>;
}

#[cfg(test)]
pub use fake::FakeClipboard;

#[cfg(test)]
mod fake {
    use super::ClipboardSource;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A clipboard the test controls. Clones share the same content.
    #[derive(Clone, Default)]
    pub struct FakeClipboard(Rc<RefCell<Option<String>>>);

    impl FakeClipboard {
        pub fn set(&self, text: Option<&str>) {
            *self.0.borrow_mut() = text.map(str::to_string);
        }
    }

    impl ClipboardSource for FakeClipboard {
        fn text(&self) -> Option<String> {
            self.0.borrow().clone()
        }
    }
}
