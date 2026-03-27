# queson

[![CI](https://github.com/cdhowie/queson/actions/workflows/CI.yml/badge.svg)](https://github.com/cdhowie/queson/actions/workflows/CI.yml)

An experimental JSON encoder/decoder Python module written in Rust.

Goals:

* [x] Faster than Python's built-in `json` module, with at least comparable RAM
  usage.
* [x] Only Rust code.
* [x] Transparent support for arbitrary-precision integers, without encoding
  them as strings.
* [x] During encoding and decoding, support for a custom "object hook" function.
    * When encoding, the object hook is called on any value that is unsupported
      by the encoder.  If the hook returns a value that is supported, encoding
      proceeds with that value instead.
    * When decoding, the object hook is called with each produced `dict` value.
      The value returned by the function takes the place of the `dict` in the
      decoded object graph.
* [x] During encoding, support for "fragment" values, which represent
  already-JSON-encoded strings that should be dumped verbatim in the output.

# Benchmark

The following results were collected using the `benchmarks` directory in this
repository.  The documents tested are real-world messages collected from the
[Archipelago](https://github.com/ArchipelagoMW/Archipelago) client.

Benchmark environment:

* Debian Trixie (Linux kernel 6.19.6)
* AMD Ryzen 9 3900X with 32GB RAM
* Python 3.11.11
    * orjson 3.9.5

| Benchmark                          | json    | queson                | orjson                 |
| :--------------------------------- | ------: | --------------------: | ---------------------: |
| testfiles/jsonmsg-1.json loads     | 177 us  | 86.7 us: 2.04x faster | 52.8 us: 3.34x faster  |
| testfiles/jsonmsg-1.json dumps     | 213 us  | 67.2 us: 3.16x faster | 21.1 us: 10.07x faster |
| testfiles/jsonmsg-23.json loads    | 2.29 ms | 1.53 ms: 1.50x faster | 785 us: 2.92x faster   |
| testfiles/jsonmsg-23.json dumps    | 2.66 ms | 575 us: 4.61x faster  | 236 us: 11.26x faster  |
| testfiles/jsonmsg-5.json loads     | 897 us  | 879 us: 1.02x faster  | 522 us: 1.72x faster   |
| testfiles/jsonmsg-5.json dumps     | 1.24 ms | 395 us: 3.13x faster  | 225 us: 5.51x faster   |
| testfiles/jsonmsg-7.json loads     | 664 us  | 523 us: 1.27x faster  | 295 us: 2.25x faster   |
| testfiles/jsonmsg-7.json dumps     | 865 us  | 180 us: 4.81x faster  | 73.7 us: 11.73x faster |
| testfiles/oops-all-ints.json loads | 168 us  | 80.5 us: 2.09x faster | 45.6 us: 3.69x faster  |
| testfiles/oops-all-ints.json dumps | 203 us  | 65.2 us: 3.11x faster | 20.1 us: 10.10x faster |
| Geometric mean                     | (ref)   | 2.37x faster          | 5.03x faster           |

Running the same benchmarks but monitoring memory usage concludes that `queson`
has a 2% higher RSS peak than `json`, and `orjson` has a 13% higher RSS peak
than `json`.

# Differences from Python's `json` module

This list may not be exhaustive.

* There is currently no streaming support (`load` and `dump` are absent).  This
  may be added in the future.
* For efficiency, `dumps` produces a UTF-8 encoded `bytes` instead of a `str`.
  The rationale is that the return value is often immediately written to a file
  or network socket anyway.
* Non-finite float values (`NaN`, `Infinity`, `-Infinity`) are rejected during
  encoding and decoding as they are not valid JSON.
* `dumps` does not support `float` `dict` keys.  The JSON specification does not
  guarantee a particular method of formatting float values, nor does it
  guarantee any specific level of precision.  The lack of a canonical float
  representation means `float` keys are of dubious value.
* `loads` does not support `bytearray` objects.  This is because they are
  mutable, and object hook support would allow Python code to mutate the
  contents while the decoder is running.  As this can invalidate the data
  pointer, it would be necessary to re-obtain the data pointer after every
  object hook invocation for soundness.

# Compliance

Passes the following test suites:

* https://github.com/nst/JSONTestSuite
