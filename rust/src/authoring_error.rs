//! Feature-independent errors returned by DOCX and PDF authoring operations.

use crate::EngineError;

#[derive(Debug, thiserror::Error)]
pub enum AuthoringError {
    #[error("{0}")]
    Value(String),
    #[error("{0}")]
    Index(String),
    #[error("{0}")]
    Key(String),
    #[error("{0}")]
    Runtime(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

pub type AuthoringResult<T> = Result<T, AuthoringError>;

macro_rules! error_constructor {
    ($name:ident, $variant:ident) => {
        pub(crate) struct $name;
        impl $name {
            pub(crate) fn new_err(message: impl Into<String>) -> AuthoringError {
                AuthoringError::$variant(message.into())
            }
        }
    };
}
error_constructor!(ValueError, Value);
error_constructor!(IndexError, Index);
error_constructor!(KeyError, Key);
error_constructor!(RuntimeError, Runtime);

#[cfg(feature = "python")]
impl From<AuthoringError> for pyo3::PyErr {
    fn from(error: AuthoringError) -> Self {
        match error {
            AuthoringError::Value(message) => pyo3::exceptions::PyValueError::new_err(message),
            AuthoringError::Index(message) => pyo3::exceptions::PyIndexError::new_err(message),
            AuthoringError::Key(message) => pyo3::exceptions::PyKeyError::new_err(message),
            AuthoringError::Runtime(message) => pyo3::exceptions::PyRuntimeError::new_err(message),
            AuthoringError::Engine(error) => error.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_error_categories_without_python() {
        assert!(matches!(
            ValueError::new_err("value"),
            AuthoringError::Value(_)
        ));
        assert!(matches!(
            IndexError::new_err("index"),
            AuthoringError::Index(_)
        ));
        assert!(matches!(KeyError::new_err("key"), AuthoringError::Key(_)));
        assert!(matches!(
            RuntimeError::new_err("runtime"),
            AuthoringError::Runtime(_)
        ));
        assert!(matches!(
            AuthoringError::from(EngineError::Encrypted),
            AuthoringError::Engine(EngineError::Encrypted)
        ));
    }

    #[cfg(feature = "python")]
    #[test]
    fn maps_to_existing_python_exception_kinds() {
        use pyo3::{exceptions::*, Python};
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            assert!(
                pyo3::PyErr::from(ValueError::new_err("value")).is_instance_of::<PyValueError>(py)
            );
            assert!(
                pyo3::PyErr::from(IndexError::new_err("index")).is_instance_of::<PyIndexError>(py)
            );
            assert!(pyo3::PyErr::from(KeyError::new_err("key")).is_instance_of::<PyKeyError>(py));
            assert!(pyo3::PyErr::from(RuntimeError::new_err("runtime"))
                .is_instance_of::<PyRuntimeError>(py));
            assert!(
                pyo3::PyErr::from(AuthoringError::from(EngineError::InvalidArgument(
                    "value".into()
                )))
                .is_instance_of::<PyValueError>(py)
            );
        });
    }
}
