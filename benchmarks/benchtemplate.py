import pyperf

import pathlib

def run_bench(load, dump):
    runner = pyperf.Runner()

    for entry in pathlib.Path('testfiles').iterdir():
        if entry.is_file():
            with open(entry, 'rb') as f:
                encoded = f.read()

            decoded = load(encoded)

            runner.bench_func(f"{entry.name} load", load, encoded)
            runner.bench_func(f"{entry.name} dump", dump, decoded)
