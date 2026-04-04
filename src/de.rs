use std::{convert::Infallible, num::NonZeroUsize};

use pyo3::{
    exceptions::PyValueError,
    ffi::PyLong_FromString,
    prelude::*,
    types::{PyBool, PyDict, PyFloat, PyFunction, PyInt, PyList, PyNone, PyString},
};

use crate::{simd::str_find_special_byte, thunk_try};

type ThunkResult<D> = crate::thunk::ThunkResult<
    <D as Deserialization>::Any,
    ParseError<<D as Deserialization>::Error>,
    Thunk<D>,
>;

/// A parsing thunk.
///
/// This enum holds the state of an in-progress list or map parse.  These thunks
/// can be held in a heap-allocated stack to avoid a stack overflow when parsing
/// very deep structures.
///
/// It is implied that the next operation to be performed is "parse any value
/// from the input."  Once a single value is parsed, the last-encountered thunk
/// is resumed.
enum Thunk<D: Deserialization> {
    /// Incomplete parsing of a list.
    ParsingList(D::BuildingList),

    /// Incomplete parsing of a map.
    ParsingMap {
        /// Map being parsed.
        dict: D::Map,
        /// The next key to be added.
        key: D::String,
    },

    Done,
}

/// A JSON parse error.
pub enum ParseError<E> {
    /// Encountered EOF inside of a value.
    Eof,
    /// There were trailing non-whitespace characters.
    ExpectedEof,
    /// A specific token was expected.
    Expected(&'static str),
    /// Some component of a list item was expected.
    ExpectedListItem,
    /// Some component of a map item was expected.
    ExpectedMapItem,
    /// An ASCII digit was expected.
    ExpectedDigit,
    /// The character encountered is an invalid start for any JSON value.
    ExpectedAny,
    /// An invalid UTF-8 sequence was encountered while parsing a string.
    InvalidUtf8,
    /// An invalid string escape sequence was encountered.
    InvalidStringEscape,
    /// A number could not be parsed.
    InvalidNumber,
    /// A string contained an unescaped control character.
    UnescapedControlCharacter,
    /// The depth limit provided by the caller was exceeded.
    DepthLimitExceeded,
    /// Something else happened.
    ///
    /// In most cases this will be a `PyErr` that resulted from interacting with
    /// the Python API, or was returned from a user-supplied function.
    Custom(E),
}

impl From<PyErr> for ParseError<PyErr> {
    fn from(value: PyErr) -> Self {
        Self::Custom(value)
    }
}

impl<E: Into<PyErr>> From<ParseError<E>> for PyErr {
    fn from(value: ParseError<E>) -> Self {
        PyErr::new::<PyValueError, _>(match value {
            ParseError::Eof => "unexpected EOF",
            ParseError::ExpectedEof => "expected EOF",
            ParseError::ExpectedListItem => "expected list item",
            ParseError::ExpectedMapItem => "expected map item",
            ParseError::ExpectedDigit => "expected a decimal digit",
            ParseError::ExpectedAny => "expected a JSON value",
            ParseError::InvalidUtf8 => "invalid UTF-8 encoding",
            ParseError::InvalidStringEscape => "invalid string escape sequence",
            ParseError::InvalidNumber => "invalid number",
            ParseError::DepthLimitExceeded => "depth limit exceeded",
            ParseError::UnescapedControlCharacter => "unescaped control character in string",

            ParseError::Expected(s) => {
                return PyErr::new::<PyValueError, _>(format!("expected {s:?}"));
            }

            ParseError::Custom(e) => return e.into(),
        })
    }
}

impl<E: Into<PyErr>> ParseError<E> {
    /// Convert this parsing error into a `PyErr` with a note that specifies the
    /// given location as the source of the error.
    #[cfg_attr(not(Py_3_11), expect(unused_variables))]
    fn into_pyerr_with_location<'py>(self, python: Python<'py>, location: usize) -> PyErr {
        let e: PyErr = self.into();

        // Disregard any error from add_note; we have no feasible way to
        // handle it, and it shouldn't happen anyway.
        #[cfg(Py_3_11)]
        e.add_note(python, format!("at input location {location}"))
            .ok();

        e
    }
}

/// Defines what kind of values are deserialized and how.
trait Deserialization {
    /// A type that can hold any possible deserialized value.
    type Any;

    /// The null type.
    type Null: Into<Self::Any>;
    /// The boolean type.
    type Bool: Into<Self::Any>;
    /// The string type.
    type String: Into<Self::Any>;
    /// The number type.
    type Number: Into<Self::Any>;
    /// The map type.
    type Map: Into<Self::Any>;
    /// The type of a list being built.
    type BuildingList;
    /// The list type.
    type List: Into<Self::Any>;

    /// The type for custom errors, if any.
    type Error;

    /// Create a null value.
    fn create_null(&self) -> Self::Null;

    /// Create the given boolean value.
    fn create_bool(&self, value: bool) -> Self::Bool;

    /// Create the given string value.
    ///
    /// The given value is not guaranteed to be valid UTF-8.
    fn create_string(&self, value: &[u8]) -> Result<Self::String, ParseError<Self::Error>>;

    /// Create the given number value.
    ///
    /// `is_float` will be true if `value` contains at least one of: `'.'`,
    /// `'e'`, or `'E'`.
    fn create_number(
        &self,
        value: &str,
        is_float: bool,
    ) -> Result<Self::Number, ParseError<Self::Error>>;

    /// Create an empty map.
    fn create_map(&self) -> Self::Map;

    /// Extend the given map with the provided key-value pair.
    ///
    /// The given key is not guaranteed to be valid UTF-8.
    fn extend_map(
        &self,
        map: &mut Self::Map,
        key: Self::String,
        value: Self::Any,
    ) -> Result<(), ParseError<Self::Error>>;

    /// Perform any final transformations on a complete map.
    fn finish_map(&self, map: Self::Map) -> Result<Self::Any, ParseError<Self::Error>>;

    /// Create an empty list.
    fn create_list(&self) -> Self::BuildingList;

    /// Extend the given list with the provided value.
    fn extend_list(&self, list: &mut Self::BuildingList, value: Self::Any);

    /// Finish building a list.
    fn finish_list(&self, list: Self::BuildingList) -> Result<Self::List, ParseError<Self::Error>>;
}

/// Wrapper type for any Python value.
///
/// This primarily exists to support implementing `From<Bound<'py, T>>` to
/// satisfy the `Into` bounds on the associated types of [`Deserialization`].
#[repr(transparent)]
struct BoundAny<'py>(Bound<'py, PyAny>);

impl<'py, T> From<Bound<'py, T>> for BoundAny<'py> {
    fn from(value: Bound<'py, T>) -> Self {
        Self(value.into_any())
    }
}

/// Deserialization to Python values.
struct PyDeserialization<'py> {
    python: Python<'py>,
    object_hook: Option<&'py Bound<'py, PyFunction>>,
}

impl<'py> Deserialization for PyDeserialization<'py> {
    type Any = BoundAny<'py>;

    type Null = Bound<'py, PyNone>;
    type Bool = Bound<'py, PyBool>;
    type String = Bound<'py, PyString>;
    // Can be float or int, so must be any.
    type Number = BoundAny<'py>;
    type Map = Bound<'py, PyDict>;
    type BuildingList = Vec<BoundAny<'py>>;
    type List = Bound<'py, PyList>;

    type Error = PyErr;

    fn create_null(&self) -> Self::Null {
        PyNone::get(self.python).to_owned()
    }

    fn create_bool(&self, value: bool) -> Self::Bool {
        PyBool::new(self.python, value).to_owned()
    }

    fn create_string(&self, value: &[u8]) -> Result<Self::String, ParseError<Self::Error>> {
        Ok(PyString::from_bytes(self.python, value)?)
    }

    fn create_number(
        &self,
        value: &str,
        is_float: bool,
    ) -> Result<Self::Number, ParseError<Self::Error>> {
        match is_float {
            false => {
                // Try parsing as a 64-bit integer first; this is significantly
                // faster than using the Python int type constructor.
                //
                // We will use that constructor if parsing fails here in order
                // to support numbers that don't fit in 64 bits.

                if value.starts_with('-') {
                    if let Ok(parsed) = value.parse::<i64>() {
                        return Ok(PyInt::new(self.python, parsed).into());
                    }
                } else if let Ok(parsed) = value.parse::<u64>() {
                    return Ok(PyInt::new(self.python, parsed).into());
                }

                // To parse ints that don't fit into i64 or u64 we will
                // construct a nul-terminated string and use PyLong_FromString.
                //
                // Using the int type constructor instead via PyO3 is half as
                // fast in benchmarks.
                let mut s = Vec::with_capacity(value.len() + 1);
                s.extend(value.as_bytes());
                s.push(0);

                // SAFETY: We give PyLong_FromString a valid pointer to a
                // nul-terminated string of ASCII digits, it will return a null
                // pointer on failure, and Bound::from_owned_ptr_or_err will
                // check the null case for us.
                unsafe {
                    Ok(Bound::from_owned_ptr_or_err(
                        self.python,
                        PyLong_FromString(s.as_ptr().cast(), std::ptr::null_mut(), 10),
                    )?
                    .into())
                }
            }

            true => {
                let parsed: f64 = value
                    .parse()
                    .map_err(|_| ParseError::<PyErr>::InvalidNumber)?;

                match parsed.is_finite() {
                    true => Ok(PyFloat::new(self.python, parsed).into()),
                    false => Err(ParseError::InvalidNumber),
                }
            }
        }
    }

    fn create_map(&self) -> Self::Map {
        PyDict::new(self.python)
    }

    fn extend_map(
        &self,
        map: &mut Self::Map,
        key: Self::String,
        value: Self::Any,
    ) -> Result<(), ParseError<Self::Error>> {
        Ok(map.set_item(key, value.0)?)
    }

    fn finish_map(&self, map: Self::Map) -> Result<Self::Any, ParseError<Self::Error>> {
        match &self.object_hook {
            None => Ok(map.into()),
            Some(hook) => Ok(hook.call1((map,)).map(|r| r.into())?),
        }
    }

    fn create_list(&self) -> Self::BuildingList {
        vec![]
    }

    fn extend_list(&self, list: &mut Self::BuildingList, value: Self::Any) {
        list.push(value);
    }

    fn finish_list(&self, list: Self::BuildingList) -> Result<Self::List, ParseError<Self::Error>> {
        Ok(PyList::new(self.python, list.into_iter().map(|a| a.0))?)
    }
}

/// Deserialization to nothing.
///
/// This type of deserialization can be used to validate that an input is valid
/// JSON without the overhead of producing a value.
struct ValidateDeserialization;

impl Deserialization for ValidateDeserialization {
    type Any = ();
    type Null = ();
    type Bool = ();
    type String = ();
    type Number = ();
    type Map = ();
    type BuildingList = ();
    type List = ();

    type Error = Infallible;

    fn create_null(&self) -> Self::Null {}

    fn create_bool(&self, _value: bool) -> Self::Bool {}

    fn create_string(&self, value: &[u8]) -> Result<Self::String, ParseError<Self::Error>> {
        str::from_utf8(value).map_err(|_| ParseError::InvalidUtf8)?;

        Ok(())
    }

    fn create_number(
        &self,
        value: &str,
        is_float: bool,
    ) -> Result<Self::Number, ParseError<Self::Error>> {
        match is_float {
            true => match value.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok(()),
                _ => Err(ParseError::InvalidNumber),
            },

            // If is_float is false, the string is guaranteed to only be ASCII
            // digits, which makes it a valid Python int.
            false => Ok(()),
        }
    }

    fn create_map(&self) -> Self::Map {}

    fn extend_map(
        &self,
        _map: &mut Self::Map,
        _key: Self::String,
        _value: Self::Any,
    ) -> Result<(), ParseError<Self::Error>> {
        Ok(())
    }

    fn finish_map(&self, _map: Self::Map) -> Result<Self::Any, ParseError<Self::Error>> {
        Ok(())
    }

    fn create_list(&self) -> Self::List {}

    fn extend_list(&self, _list: &mut Self::BuildingList, _value: Self::Any) {}

    fn finish_list(
        &self,
        _list: Self::BuildingList,
    ) -> Result<Self::List, ParseError<Self::Error>> {
        Ok(())
    }
}

/// Byte-slice cursor.
///
/// This trait makes working with `&mut &[u8]` more ergonomic, as well as
/// provides a starting point to handle e.g. streaming deserialization later.
trait Cursor {
    /// Look at the first byte, returning an EOF error if there is none.
    fn peek<T>(&self) -> Result<u8, ParseError<T>>;

    /// Skips to the next byte.
    ///
    /// # Panics
    ///
    /// This may panic if the cursor is viewing an empty slice.  This method
    /// should not be used unless you have already verified that the slice is
    /// not empty.
    fn skip(&mut self) {
        self.skip_n(1);
    }

    /// Skips over the given number of bytes.
    ///
    /// # Panics
    ///
    /// This may panic if the cursor is viewing a slice of less than the
    /// provided number of bytes.  This method should not be used unless you
    /// have already verified that the slice contains at least as many bytes as
    /// are being skipped.
    fn skip_n(&mut self, n: usize);

    /// Reads the next byte, returning an EOF error if there is none.
    fn read<T>(&mut self) -> Result<u8, ParseError<T>>;

    /// Consumes all characters considered whitespace by the JSON specification.
    ///
    /// After calling this method, the byte slice must be either empty, or the
    /// first character must not be whitespace.
    fn consume_whitespace(&mut self);
}

impl Cursor for &[u8] {
    fn peek<T>(&self) -> Result<u8, ParseError<T>> {
        self.first().copied().ok_or(ParseError::Eof)
    }

    fn skip_n(&mut self, n: usize) {
        *self = &self[n..];
    }

    fn read<T>(&mut self) -> Result<u8, ParseError<T>> {
        let v = self.peek()?;
        self.skip();
        Ok(v)
    }

    fn consume_whitespace(&mut self) {
        for i in 0..self.len() {
            if !matches!(self[i], b' ' | b'\n' | b'\r' | b'\t') {
                *self = &self[i..];
                return;
            }
        }

        *self = &[];
    }
}

/// Helper trait for the [`expect`] function.
trait Expect {
    /// Expect to see `self` at the start of the provided slice.
    ///
    /// If `b` begins with the value of `self`, `b` is adjusted to skip over
    /// that value, and `true` is returned.  Otherwise, `false` is returned and
    /// `b` is not adjusted.
    fn expect(self, b: &mut &[u8]) -> bool;
}

impl<const N: usize> Expect for &[u8; N] {
    fn expect(self, b: &mut &[u8]) -> bool {
        if b.starts_with(self) {
            b.skip_n(N);
            true
        } else {
            false
        }
    }
}

impl Expect for u8 {
    fn expect(self, b: &mut &[u8]) -> bool {
        if b.peek::<()>().is_ok_and(|v| v == self) {
            b.skip();
            true
        } else {
            false
        }
    }
}

/// Expect a given value to be present at the beginning of the provided slice.
///
/// If `b` begins with `expected`, advances `b` past that value and returns `Ok(())`.
///
/// Otherwise, returns `Err(err())`.
fn expect<T>(
    b: &mut &[u8],
    expected: impl Expect,
    err: impl FnOnce() -> ParseError<T>,
) -> Result<(), ParseError<T>> {
    match expected.expect(b) {
        true => Ok(()),
        false => Err(err()),
    }
}

/// Parse the next JSON value from the provided slice.
fn parse_any<D: Deserialization>(deserialization: &D, b: &mut &[u8]) -> ThunkResult<D> {
    ThunkResult::<D>::Ok(match thunk_try!(b.peek()) {
        b'n' => {
            b.skip();
            thunk_try!(expect(b, b"ull", || ParseError::Expected("null")));
            deserialization.create_null().into()
        }

        b'f' => {
            b.skip();
            thunk_try!(expect(b, b"alse", || ParseError::Expected("false")));
            deserialization.create_bool(false).into()
        }

        b't' => {
            b.skip();
            thunk_try!(expect(b, b"rue", || ParseError::Expected("true")));
            deserialization.create_bool(true).into()
        }

        b'"' => {
            b.skip();
            thunk_try!(parse_str(deserialization, b).map_err(|e| *e)).into()
        }

        b'[' => {
            b.skip();
            return parse_list(deserialization, b);
        }

        b'{' => {
            b.skip();
            return parse_map(deserialization, b);
        }

        b'-' | b'0'..=b'9' => thunk_try!(parse_number(deserialization, b)).into(),

        _ => return ThunkResult::<D>::Err(ParseError::ExpectedAny),
    })
}

/// Parse a number from the provided slice.
#[allow(clippy::manual_is_ascii_check)]
fn parse_number<D: Deserialization>(
    deserialization: &D,
    b: &mut &[u8],
) -> Result<D::Number, ParseError<D::Error>> {
    let start = *b;
    let mut is_float = false;

    let c = match b.read()? {
        b'-' => b.read()?,

        c => c,
    };

    match c {
        b'1'..=b'9' => {
            while matches!(b.first(), Some(b'0'..=b'9')) {
                b.skip();
            }
        }

        b'0' => {}

        _ => return Err(ParseError::ExpectedDigit),
    }

    if matches!(b.peek::<()>(), Ok(b'.')) {
        b.skip();
        is_float = true;

        if !matches!(b.read()?, b'0'..=b'9') {
            return Err(ParseError::ExpectedDigit);
        }

        while matches!(b.first(), Some(b'0'..=b'9')) {
            b.skip();
        }
    }

    if matches!(b.first(), Some(b'E' | b'e')) {
        b.skip();
        is_float = true;

        if matches!(b.first(), Some(b'-' | b'+')) {
            b.skip();
        }

        if !matches!(b.read()?, b'0'..=b'9') {
            return Err(ParseError::ExpectedDigit);
        }

        while matches!(b.first(), Some(b'0'..=b'9')) {
            b.skip();
        }
    }

    let bytes = start.len() - b.len();

    let number = &start[0..bytes];

    // SAFETY: The byte slice only contains ASCII characters, otherwise we would
    // have already returned Err somewhere above.
    let number = unsafe { str::from_utf8_unchecked(number) };

    deserialization.create_number(number, is_float)
}

/// Parse a string from the front of the slice.
///
/// This function should be called when `b` has already been advanced past the
/// `"` character that began the string.
fn parse_str<D: Deserialization>(
    deserialization: &D,
    b: &mut &[u8],
) -> Result<D::String, Box<ParseError<D::Error>>> {
    // The return error type is boxed as this function does not get inlined.
    // Boxing it changes the return value size from 56 bytes to 16, allowing it
    // to fit into registers.

    let start = *b;

    // Skip as many characters as we can using a vectorized search.
    b.skip_n(str_find_special_byte(b));

    // Start under the assumption that we can borrow the encoded string.  The
    // only thing that can make this impossible is escape sequences.
    let mut buf = loop {
        match b.peek()? {
            b'"' => {
                let bytes = start.len() - b.len();
                b.skip();

                return Ok(deserialization.create_string(&start[0..bytes])?);
            }

            b'\\' => {
                let bytes = start.len() - b.len();

                // Convert what we've already read into an owned byte string and
                // fall down to the next loop, which handles building the rest
                // of the owned string.
                break start[0..bytes].to_owned();
            }

            c if c < b' ' => return Err(ParseError::UnescapedControlCharacter.into()),

            _ => {
                b.skip();
            }
        }
    };

    loop {
        match b.read()? {
            b'\\' => match b.read()? {
                b'b' => buf.push(b'\x08'),
                b'f' => buf.push(b'\x0C'),
                b'n' => buf.push(b'\n'),
                b'r' => buf.push(b'\r'),
                b't' => buf.push(b'\t'),

                b'u' => {
                    let c1 = parse_unicode_escape(b)?;

                    let codepoint = match char::from_u32(c1.into()) {
                        // char::from_u32 fails on surrogates, so if this
                        // succeeds we are done.
                        Some(c) => c,

                        None => match c1 {
                            // Leading surrogate.
                            0xD800..=0xDBFF => {
                                expect(b, b"\\u", || ParseError::InvalidStringEscape)?;

                                let c2 = parse_unicode_escape(b)?;

                                if !matches!(c2, 0xDC00..=0xDFFF) {
                                    // The next character was not a trailing
                                    // surrogate, so this is not a valid
                                    // surrogate pair.
                                    return Err(ParseError::InvalidStringEscape.into());
                                }

                                char::from_u32(
                                    (u32::from(c1 - 0xD800) << 10)
                                        + u32::from(c2 - 0xDC00)
                                        + 0x10000,
                                )
                                .ok_or(ParseError::InvalidStringEscape)?
                            }

                            // Trailing surrogate without leading surrogate.
                            0xDC00..=0xDFFF => return Err(ParseError::InvalidStringEscape.into()),

                            // from_u32 should have returned Some in this case.
                            _ => unreachable!(),
                        },
                    };

                    let mut utf8_buf = [0; 4];

                    buf.extend(codepoint.encode_utf8(&mut utf8_buf).as_bytes());
                }

                c @ (b'\\' | b'/' | b'"') => buf.push(c),

                _ => return Err(ParseError::InvalidStringEscape.into()),
            },

            b'"' => {
                return Ok(deserialization.create_string(&buf)?);
            }

            c if c < b' ' => return Err(ParseError::UnescapedControlCharacter.into()),

            c => buf.push(c),
        };
    }
}

/// Parses a unicode escape sequence.
///
/// The buffer should already be advanced past the `\u` sequence so that the
/// first 4 bytes of the slice are expected to be hexadecimal digits.
fn parse_unicode_escape<T>(b: &mut &[u8]) -> Result<u16, ParseError<T>> {
    if b.len() < 4 {
        return Err(ParseError::Eof);
    };

    let hex = str::from_utf8(&b[0..4]).map_err(|_| ParseError::InvalidUtf8)?;

    if hex.len() != 4 {
        // If the length is not 4 then there was at least one non-ASCII
        // character, so they can't all be hex digits.
        return Err(ParseError::InvalidStringEscape);
    }

    b.skip_n(4);

    u16::from_str_radix(hex, 16).map_err(|_| ParseError::InvalidStringEscape)
}

/// Parse a list from the front of the slice.
///
/// This function should be called when `b` has already been advanced past the
/// `[` character that began the list.
fn parse_list<D: Deserialization>(deserialization: &D, b: &mut &[u8]) -> ThunkResult<D> {
    let list = deserialization.create_list();

    b.consume_whitespace();

    if thunk_try!(b.peek()) == b']' {
        b.skip();
        return ThunkResult::<D>::Ok(thunk_try!(deserialization.finish_list(list)).into());
    }

    ThunkResult::Thunk(Thunk::ParsingList(list))
}

/// Continue parsing a list.
///
/// This is called once the next value of a list has been parsed, and parsing
/// the list itself is being resumed from a thunk.
fn continue_parse_list<D: Deserialization>(
    deserialization: &D,
    mut list: D::BuildingList,
    value: D::Any,
    b: &mut &[u8],
) -> ThunkResult<D> {
    deserialization.extend_list(&mut list, value);

    b.consume_whitespace();

    match thunk_try!(b.read()) {
        b']' => ThunkResult::Ok(thunk_try!(deserialization.finish_list(list)).into()),

        b',' => {
            b.consume_whitespace();
            ThunkResult::Thunk(Thunk::ParsingList(list))
        }

        _ => ThunkResult::Err(ParseError::ExpectedListItem),
    }
}

/// Parse a map from the front of the slice.
///
/// This function should be called when `b` has already been advanced past the
/// `{` character that began the map.
fn parse_map<D: Deserialization>(deserialization: &D, b: &mut &[u8]) -> ThunkResult<D> {
    let dict = deserialization.create_map();

    b.consume_whitespace();

    let key = match thunk_try!(b.read()) {
        b'}' => {
            return ThunkResult::<D>::Ok(thunk_try!(deserialization.finish_map(dict)));
        }

        b'"' => thunk_try!(parse_str(deserialization, b).map_err(|e| *e)),

        _ => return ThunkResult::<D>::Err(ParseError::ExpectedMapItem),
    };

    b.consume_whitespace();
    thunk_try!(expect(b, b':', || ParseError::ExpectedMapItem));
    b.consume_whitespace();

    ThunkResult::Thunk(Thunk::ParsingMap { dict, key })
}

/// Continue parsing a map.
///
/// This is called once the next value of a map has been parsed, and parsing the
/// map itself is being resumed from a thunk.
fn continue_parse_map<D: Deserialization>(
    deserialization: &D,
    mut dict: D::Map,
    key: D::String,
    value: D::Any,
    b: &mut &[u8],
) -> ThunkResult<D> {
    thunk_try!(deserialization.extend_map(&mut dict, key, value));

    b.consume_whitespace();

    match thunk_try!(b.read()) {
        b'}' => ThunkResult::Ok(thunk_try!(deserialization.finish_map(dict))),

        b',' => {
            b.consume_whitespace();
            thunk_try!(expect(b, b'"', || ParseError::ExpectedMapItem));

            let key = thunk_try!(parse_str(deserialization, b).map_err(|e| *e));

            b.consume_whitespace();
            thunk_try!(expect(b, b':', || ParseError::ExpectedMapItem));
            b.consume_whitespace();

            ThunkResult::Thunk(Thunk::ParsingMap { dict, key })
        }

        _ => ThunkResult::Err(ParseError::ExpectedMapItem),
    }
}

/// Parse a JSON value with the given [`Deserialization`] implementation.
fn parse_json_with<D: Deserialization>(
    deserialization: &D,
    depth_limit: Option<NonZeroUsize>,
    mut json: &[u8],
) -> Result<D::Any, (ParseError<D::Error>, usize)> {
    let len = json.len();

    json.consume_whitespace();

    let mut stack = vec![];

    let mut last_any = ThunkResult::Thunk(Thunk::Done);

    let result = loop {
        last_any = match last_any {
            ThunkResult::Err(e) => {
                return Err((e, len - json.len()));
            }

            ThunkResult::Thunk(op) => {
                stack.push(op);

                if depth_limit.is_some_and(|limit| stack.len() >= limit.into()) {
                    return Err((ParseError::DepthLimitExceeded, len - json.len()));
                }

                parse_any(deserialization, &mut json)
            }

            ThunkResult::Ok(value) => match stack.pop().unwrap() {
                Thunk::ParsingList(list) => {
                    continue_parse_list(deserialization, list, value, &mut json)
                }

                Thunk::ParsingMap { dict, key } => {
                    continue_parse_map(deserialization, dict, key, value, &mut json)
                }

                Thunk::Done => break value,
            },
        };
    };

    json.consume_whitespace();

    match json.is_empty() {
        true => Ok(result),
        false => Err((ParseError::ExpectedEof, len - json.len())),
    }
}

/// Parse a JSON-encoded value.
///
/// `json` must contain a UTF-8 encoded string of data that represents a single
/// valid JSON value.
///
/// `object_hook` is an optional Python function that will be called on maps
/// that have been fully decoded.  The hook can return any valid Python value,
/// which will take the place of the Python dict in the deserialized object
/// graph.
pub fn parse_json<'py>(
    python: Python<'py>,
    json: &[u8],
    object_hook: Option<&'py Bound<'py, PyFunction>>,
    depth_limit: Option<NonZeroUsize>,
) -> PyResult<Bound<'py, PyAny>> {
    match parse_json_with(
        &PyDeserialization {
            python,
            object_hook,
        },
        depth_limit,
        json,
    ) {
        Ok(v) => Ok(v.0),
        Err((e, location)) => Err(e.into_pyerr_with_location(python, location)),
    }
}

/// Validates that the given JSON-encoded value is well-formed.
pub fn validate_json<'py>(
    python: Python<'py>,
    json: &[u8],
    depth_limit: Option<NonZeroUsize>,
) -> PyResult<()> {
    parse_json_with(&ValidateDeserialization, depth_limit, json)
        .map_err(|(e, location)| e.into_pyerr_with_location(python, location))
}
