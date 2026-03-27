import apjson
import json

from pathlib import Path

def test_jsontestsuite():
    testsdir = Path(__file__).resolve().parent.parent / 'external/JSONTestSuite/test_parsing'

    for file in testsdir.iterdir():
        if file.is_file():
            try:
                with open(file, 'rb') as f:
                    content = f.read()

                try:
                    result = apjson.loads(content)
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

    result = apjson.loads('{"foo":"bar","custom":{"type":"$custom","value":42}}', object_hook=hook)

    assert result == {
        "foo": "bar",
        "custom": CustomValue(42),
    }

def test_dumps_objecthook():
    def hook(v):
        if isinstance(v, CustomValue):
            return {"type": "$custom", "value": v.value}

        return v

    result = apjson.dumps({
        "foo": "bar",
        "custom": CustomValue(42),
    }, object_hook=hook)

    parsed = apjson.loads(result)

    assert parsed == {
        "foo": "bar",
        "custom": {
            "type": "$custom",
            "value": 42,
        },
    }
