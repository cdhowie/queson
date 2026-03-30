import benchtemplate

import orjson

benchtemplate.run_bench(orjson.loads, orjson.dumps)
