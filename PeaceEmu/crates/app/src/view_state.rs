#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared, explicit view states for frontend-neutral application pages.

/// A user-facing page state, independent of any specific UI toolkit.
///
/// Keeping absence, temporary loading, unavailable capability, and failure
/// distinct prevents frontends from collapsing materially different situations
/// into the same empty panel or success message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewState<T> {
    /// The page is waiting for an operation or initial data.
    Loading,
    /// Data is available and can be rendered.
    Ready(T),
    /// The operation succeeded, but there is no content to display.
    Empty,
    /// The requested operation cannot be offered in the current environment.
    Unavailable(String),
    /// The operation failed; the message should describe the failure.
    Error(String),
}

impl<T> ViewState<T> {
    /// Returns true when the state contains renderable data.
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Returns true when the operation completed successfully without content.
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns true when the state describes an unavailable capability.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }

    /// Borrows the message when the state describes an unavailable capability.
    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Unavailable(reason) => Some(reason),
            _ => None,
        }
    }

    /// Borrows the message when the state describes a failed operation.
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error(message) => Some(message),
            _ => None,
        }
    }

    /// Returns true when the state represents a failed operation.
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Borrows ready data while preserving the current state and its messages.
    pub fn as_ref(&self) -> ViewState<&T> {
        match self {
            Self::Loading => ViewState::Loading,
            Self::Ready(value) => ViewState::Ready(value),
            Self::Empty => ViewState::Empty,
            Self::Unavailable(reason) => ViewState::Unavailable(reason.clone()),
            Self::Error(message) => ViewState::Error(message.clone()),
        }
    }

    /// Maps ready data while preserving every non-ready state unchanged.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> ViewState<U> {
        match self {
            Self::Loading => ViewState::Loading,
            Self::Ready(value) => ViewState::Ready(map(value)),
            Self::Empty => ViewState::Empty,
            Self::Unavailable(reason) => ViewState::Unavailable(reason),
            Self::Error(message) => ViewState::Error(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ViewState;

    #[test]
    fn page_states_remain_distinct_when_mapping_data() {
        let states = [
            ViewState::<u8>::Loading,
            ViewState::Empty,
            ViewState::Unavailable("audio device missing".to_owned()),
            ViewState::Error("preferences could not be saved".to_owned()),
        ];

        let expected = [
            ViewState::<String>::Loading,
            ViewState::Empty,
            ViewState::Unavailable("audio device missing".to_owned()),
            ViewState::Error("preferences could not be saved".to_owned()),
        ];

        for (state, expected) in states.into_iter().zip(expected) {
            let mapped = state.map(|value| value.to_string());
            assert!(!mapped.is_ready());
            assert_eq!(mapped, expected);
        }

        let ready = ViewState::Ready(42_u8).map(|value| value.to_string());
        assert_eq!(ready, ViewState::Ready("42".to_owned()));
    }

    #[test]
    fn error_and_unavailable_messages_are_borrowable_without_cloning() {
        let unavailable = ViewState::<u8>::Unavailable("audio device missing".to_owned());
        let error = ViewState::<u8>::Error("preferences write failed".to_owned());

        assert_eq!(
            unavailable.unavailable_reason(),
            Some("audio device missing")
        );
        assert_eq!(unavailable.error_message(), None);
        assert_eq!(error.error_message(), Some("preferences write failed"));
        assert_eq!(error.unavailable_reason(), None);
        assert_eq!(ViewState::<u8>::Loading.unavailable_reason(), None);
    }

    #[test]
    fn map_does_not_invoke_transform_for_non_ready_states() {
        use std::cell::Cell;

        let calls = Cell::new(0);
        let state = ViewState::<u8>::Unavailable("device missing".to_owned());
        let mapped = state.map(|value| {
            calls.set(calls.get() + 1);
            value + 1
        });

        assert_eq!(calls.get(), 0);
        assert_eq!(
            mapped,
            ViewState::<u8>::Unavailable("device missing".to_owned())
        );
    }

    #[test]
    fn status_helpers_distinguish_unavailable_from_failure() {
        let unavailable = ViewState::<u8>::Unavailable("audio device missing".to_owned());
        let error = ViewState::<u8>::Error("audio initialization failed".to_owned());

        assert!(unavailable.is_unavailable());
        assert!(!unavailable.is_error());
        assert!(error.is_error());
        assert!(!error.is_unavailable());
        assert!(!ViewState::<u8>::Empty.is_error());
    }

    #[test]
    fn borrowing_ready_data_does_not_consume_the_state() {
        let state = ViewState::Ready(vec![1_u8, 2, 3]);
        let borrowed = state.as_ref();

        assert_eq!(borrowed, ViewState::Ready(&vec![1, 2, 3]));
        assert_eq!(state, ViewState::Ready(vec![1, 2, 3]));
    }

    #[test]
    fn borrowing_preserves_unavailable_and_error_messages() {
        assert_eq!(
            ViewState::<u8>::Unavailable("no audio device".to_owned()).as_ref(),
            ViewState::<&u8>::Unavailable("no audio device".to_owned())
        );
        assert_eq!(
            ViewState::<u8>::Error("disk full".to_owned()).as_ref(),
            ViewState::<&u8>::Error("disk full".to_owned())
        );
    }

    #[test]
    fn empty_is_success_without_data_not_an_error() {
        let empty = ViewState::<Vec<String>>::Empty;
        assert!(empty.is_empty());
        assert!(!empty.is_ready());
        assert_eq!(empty, ViewState::Empty);
    }
}
