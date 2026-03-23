/// Specialized result for functions that can return thunks.
#[must_use]
pub enum ThunkResult<S, E, T> {
    /// Success.
    Ok(S),
    /// Failure.
    Err(E),

    /// Incomplete with a thunk holding the result of the operation so far.
    Thunk(T),
}

impl<S, E, T> ThunkResult<S, E, T> {
    /// Maps the error type by applying a closure.
    pub fn map_err<U>(self, f: impl FnOnce(E) -> U) -> ThunkResult<S, U, T> {
        match self {
            Self::Ok(v) => ThunkResult::Ok(v),
            Self::Err(e) => ThunkResult::Err(f(e)),
            Self::Thunk(t) => ThunkResult::Thunk(t),
        }
    }
}

/// Unwraps a standard [`Result`] like the `?` operator, but the `Err` variant
/// is repackaged into a [`ThunkResult::Err`].
///
/// This allows for more ergonomic usage of `Result`-returning functions from
/// within a function that returns `ThunkResult`.
#[macro_export]
macro_rules! thunk_try {
    ( $e:expr ) => {
        match $e {
            ::std::result::Result::Ok(v) => v,
            ::std::result::Result::Err(e) => return ThunkResult::Err(e.into()),
        }
    };
}
