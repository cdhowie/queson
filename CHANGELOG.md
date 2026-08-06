# 0.2.5

* Fixed off-by-one bug with the `depth_limit` parameter in the `load` family of
  functions.  This was a regression introduced in 0.2.3.

# 0.2.4

* Add `dump` and `load` shims.

# 0.2.3

* Optimizations.

# 0.2.2

* Fixed `depth_limit` argument missing in type stubs.

# 0.2.1

* Added `depth_limit` option to all deserialization/validation functions.

# 0.2.0

## Breaking changes

* `dumps` now returns `str` for API compatibility with other JSON modules.  The
  new `dumpb` function returns `bytes`.
* Optional arguments to all functions and constructors are now keyword-only.

## Other changes

* Added `loadb` as an alias of `loads` for API symmetry with `dumps`/`dumpb`.
  Both functions will accept either `str` or `bytes`.

# 0.1.1

* Add type stubs.

# 0.1.0

* Initial release.
