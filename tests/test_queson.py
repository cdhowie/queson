import pytest

import queson
import json
import math
import typing

from pathlib import Path

def test_jsontestsuite() -> None:
    testsdir = Path(__file__).resolve().parent.parent / 'external/JSONTestSuite/test_parsing'

    for file in testsdir.iterdir():
        if file.is_file():
            try:
                with open(file, 'rb') as f:
                    content = f.read()

                try:
                    result = queson.loadb(content)
                    err = None
                except ValueError as e:
                    err = e
                except Exception as e:
                    raise RuntimeError('raised exception was not a ValueError') from e

                if err is not None and file.name.startswith('y_'):
                    raise RuntimeError(f"parsing failed") from err

                if err is None:
                    if file.name.startswith('n_'):
                        raise RuntimeError(f"parsing succeeded when it shouldn't")

                    # Only compare to json.loads() for tests that must succeed,
                    # because json.loads() is allowed to fail for other tests.
                    if file.name.startswith('y_'):
                        assert json.loads(content) == result
            except Exception as e:
                e.add_note(f"test case {file.name}")
                raise

class CustomValue:
    def __init__(self, value: typing.Any) -> None:
        self.value = value

    def __eq__(self, other: typing.Any) -> bool:
        return isinstance(other, CustomValue) and self.value == other.value

def test_loadb_objecthook() -> None:
    def hook(v: dict[str, typing.Any]) -> typing.Any:
        if v.get('type') == '$custom':
            return CustomValue(v['value'])

        return v

    result = queson.loadb('{"foo":"bar","custom":{"type":"$custom","value":42}}', object_hook=hook)

    assert result == {
        "foo": "bar",
        "custom": CustomValue(42),
    }

def test_dumpb_objecthook() -> None:
    def hook(v: typing.Any) -> typing.Any:
        if isinstance(v, CustomValue):
            return {"type": "$custom", "value": v.value}

        return v

    result = queson.dumpb({
        "foo": "bar",
        "custom": CustomValue(42),
    }, object_hook=hook)

    parsed = queson.loadb(result)

    assert parsed == {
        "foo": "bar",
        "custom": {
            "type": "$custom",
            "value": 42,
        },
    }

def test_loadb_objecthook_passes_error() -> None:
    def hook(v: typing.Any) -> None:
        raise RuntimeError('from hook')

    with pytest.raises(RuntimeError) as e:
        queson.loadb('{}', object_hook=hook)

    assert str(e.value) == 'from hook'

def test_dumpb_objecthook_passes_error() -> None:
    def hook(v: typing.Any) -> None:
        raise RuntimeError('from hook')

    with pytest.raises(RuntimeError) as e:
        queson.dumpb(CustomValue(42), object_hook=hook)

    assert str(e.value) == 'from hook'

def test_dumpb_invalid_values_raise_valueerror() -> None:
    for case in [
        math.inf,
        -math.inf,
        math.nan,
        CustomValue(42),
        {1.2: 3},
    ]:
        with pytest.raises(ValueError):
            queson.dumpb(case)

def test_fragment_validation() -> None:
    with pytest.raises(ValueError):
        queson.Fragment(b'{')

    queson.Fragment(b'{', validate=False)

def test_fragment() -> None:
    result = queson.dumpb([
        {},
        queson.Fragment(b'[1]'),
        queson.Fragment(b'[', validate=False),
    ])

    assert result == b'[{},[1],[]'

# Fuzzing found this case: if loadb() input contains a float that overflows, it
# was resulting in infinity, which could not be serialized back successfully
# with dumpb().  loadb() was changed to raise a ValueError if parsing encounters
# a non-finite float.
def test_float_overflow() -> None:
    with pytest.raises(ValueError):
        queson.loadb(b'1e3322')

def test_deep_structure() -> None:
    source = b'[' * 1000000 + b']' * 1000000

    assert queson.dumpb(queson.loadb(source)) == source

def test_depth_limit() -> None:
    source = b'[' * 1000000 + b']' * 1000000

    with pytest.raises(ValueError):
        queson.loadb(source, depth_limit=10000)
