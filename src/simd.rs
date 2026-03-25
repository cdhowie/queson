/// Helper to vectorize an operation.
macro_rules! simd_op {
    ( $chunk:ident[0..4] $( $op:tt )+ ) => {
        (
            ($chunk[0] $( $op )+)
            | ($chunk[1] $( $op )+)
            | ($chunk[2] $( $op )+)
            | ($chunk[3] $( $op )+)
        )
    };

    ( $chunk:ident[0..8] $( $op:tt )+ ) => {
        (
            ($chunk[0] $( $op )+)
            | ($chunk[1] $( $op )+)
            | ($chunk[2] $( $op )+)
            | ($chunk[3] $( $op )+)
            | ($chunk[4] $( $op )+)
            | ($chunk[5] $( $op )+)
            | ($chunk[6] $( $op )+)
            | ($chunk[7] $( $op )+)
        )
    };
}

/// Find the position of the first byte that requires special handling.
///
/// Bytes that require special handling are `b'\\'`, `b'"'`, and anything in
/// `0..b' '`.
///
/// The return value is not guaranteed to point at such a character, but given a
/// returned value `r`, it is guaranteed that there will be no such special
/// characters in the slice `&s[..r]`.
pub fn str_find_special_byte(s: &[u8]) -> usize {
    // This function is not strictly required by serialization logic, but is an
    // optimization.  It can examine 8 characters at once to see if a special
    // character is present.  This will make it very fast to output strings that
    // don't require any escaping.
    let mut pos = 0;

    for chunk in s[pos..].chunks_exact(8) {
        if simd_op!(chunk[0..8] == b'\\')
            | simd_op!(chunk[0..8] == b'"')
            | simd_op!(chunk[0..8] < b' ')
        {
            break;
        }

        pos += 8;
    }

    if s.len() - pos >= 4 {
        let chunk = &s[pos..(pos + 4)];

        if simd_op!(chunk[0..4] == b'\\')
            | simd_op!(chunk[0..4] == b'"')
            | simd_op!(chunk[0..4] < b' ')
        {
            return pos;
        }

        pos += 4;
    }

    pos
}
