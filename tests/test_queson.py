import pytest

import queson
import json
import math

from pathlib import Path

def test_jsontestsuite():
    testsdir = Path(__file__).resolve().parent.parent / 'external/JSONTestSuite/test_parsing'

    for file in testsdir.iterdir():
        if file.is_file():
            try:
                with open(file, 'rb') as f:
                    content = f.read()

                try:
                    result = queson.loads(content)
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

                    assert json.loads(content) == result
            except Exception as e:
                e.add_note(f"test case {file.name}")
                raise

class CustomValue:
    def __init__(self, value):
        self.value = value

    def __eq__(self, other):
        return isinstance(other, CustomValue) and self.value == other.value

def test_loads_objecthook():
    def hook(v):
        if v.get('type') == '$custom':
            return CustomValue(v['value'])

        return v

    result = queson.loads('{"foo":"bar","custom":{"type":"$custom","value":42}}', object_hook=hook)

    assert result == {
        "foo": "bar",
        "custom": CustomValue(42),
    }

def test_dumps_objecthook():
    def hook(v):
        if isinstance(v, CustomValue):
            return {"type": "$custom", "value": v.value}

        return v

    result = queson.dumps({
        "foo": "bar",
        "custom": CustomValue(42),
    }, object_hook=hook)

    parsed = queson.loads(result)

    assert parsed == {
        "foo": "bar",
        "custom": {
            "type": "$custom",
            "value": 42,
        },
    }

def test_loads_objecthook_passes_error():
    def hook(v):
        raise RuntimeError('from hook')

    with pytest.raises(RuntimeError) as e:
        queson.loads('{}', object_hook=hook)

    assert str(e.value) == 'from hook'

def test_dumps_objecthook_passes_error():
    def hook(v):
        raise RuntimeError('from hook')

    with pytest.raises(RuntimeError) as e:
        queson.dumps(CustomValue(42), object_hook=hook)

    assert str(e.value) == 'from hook'

def test_dumps_invalid_values_raise_valueerror():
    for case in [
        math.inf,
        -math.inf,
        math.nan,
        CustomValue(42),
        {1.2: 3},
    ]:
        with pytest.raises(ValueError):
            queson.dumps(case)

def test_fragment_validation():
    with pytest.raises(ValueError):
        queson.Fragment(b'{')

    queson.Fragment(b'{', validate=False)

def test_fragment():
    result = queson.dumps([
        {},
        queson.Fragment(b'[1]'),
        queson.Fragment(b'[', validate=False),
    ])

    assert result == b'[{},[1],[]'
