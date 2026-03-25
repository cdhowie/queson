import pyperf

import pathlib

def run_bench(module):
    runner = pyperf.Runner()

    for entry in pathlib.Path('testfiles').iterdir():
        if entry.is_file():
            with open(entry, 'rb') as f:
                encoded = f.read()

            decoded = module.loads(encoded)

            runner.bench_func(f"{entry} loads", module.loads, encoded)
            runner.bench_func(f"{entry} dumps", module.dumps, decoded)
