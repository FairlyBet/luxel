// The error type of every fallible call. In Rust it reads as a message (it
// derefs to `str`) with a kind; across wasm-bindgen it becomes a JS `Error`
// whose `name` says the kind ("ContextLostError", "ShaderError", or "Error"),
// so JS callers get `instanceof Error`, a message and a stack rather than a
// bare string, and can tell a lost context from a broken shader.

use std::fmt;
use std::ops::Deref;

use wasm_bindgen::JsValue;

/// What kind of failure an [`Error`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ErrorKind {
    /// Anything else: bad arguments, stale handles, unsupported features.
    #[default]
    Other,
    /// The WebGL context is lost (or was, while the work ran): retry after
    /// it is restored.
    ContextLost,
    /// A shader failed to compile or a program to link; the message has the
    /// logs.
    Shader,
}

impl ErrorKind {
    /// The JS `Error.name` for this kind.
    pub fn js_name(self) -> &'static str {
        match self {
            Self::Other => "Error",
            Self::ContextLost => "ContextLostError",
            Self::Shader => "ShaderError",
        }
    }
}

/// What went wrong, as a human-readable message, and its kind.
#[derive(Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A lost-context error with `message`.
    pub fn context_lost(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ContextLost, message)
    }

    /// A shader compile / link error with `message` (the logs).
    pub fn shader(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Shader, message)
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl Deref for Error {
    type Target = str;

    fn deref(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

// Debug shows the message as is, so `expect` / `unwrap` panics stay readable.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// A handle pool with no slot left (`rswebgl_foundation::pool::PoolFull`).
impl From<rswebgl_foundation::pool::PoolFull> for Error {
    fn from(e: rswebgl_foundation::pool::PoolFull) -> Self {
        e.to_string().into()
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::new(ErrorKind::Other, s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self::new(ErrorKind::Other, s)
    }
}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.message
    }
}

impl From<Error> for JsValue {
    fn from(e: Error) -> Self {
        let js = js_sys::Error::new(&e.message);
        js.set_name(e.kind.js_name());
        js.into()
    }
}

/// `format!` into an `Error`.
macro_rules! err {
    ($($t:tt)*) => {
        $crate::error::Error::from(format!($($t)*))
    };
}
pub(crate) use err;
