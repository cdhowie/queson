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
