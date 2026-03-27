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
                except Exception as e:
                    err = e

                if err is not None and file.name.startswith('y_'):
                    raise RuntimeError(f"parsing failed") from err

                if err is None:
                    if file.name.startswith('n_'):
                        raise RuntimeError(f"parsing succeeded when it shouldn't")

                    assert json.loads(content) == result
            except Exception as e:
                e.add_note(f"test case {file.name}")
                raise
