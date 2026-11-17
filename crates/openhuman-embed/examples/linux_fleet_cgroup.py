"""Record the release example and its enclosing Linux cgroup's limits/peak.

Run through systemd-run --user --scope; build the Rust example first.
The cgroup contains the example's HTTP mock and this small Python monitor.
"""

import json, pathlib, subprocess, sys
count=sys.argv[1]
binary = pathlib.Path(__file__).resolve().parents[3] / 'target/release/examples/linux_fleet'
r=subprocess.run([str(binary), count],capture_output=True,text=True)
if r.returncode:
 print(r.stderr, file=sys.stderr);sys.exit(r.returncode)
record=json.loads(next(line for line in r.stdout.splitlines() if line.startswith("{")))
path=next(line.split('::',1)[1] for line in pathlib.Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
cg=pathlib.Path('/sys/fs/cgroup')/path.lstrip('/')
for name in ['memory.peak','memory.max','cpu.max','memory.events']:
 record['cgroup_'+name.replace('.','_')]=(cg/name).read_text().strip()
print(json.dumps(record))
