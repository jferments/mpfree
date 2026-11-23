#!/bin/sh
# Report only: how hard the CI VM's mpfree slice is being held back, read from
# its cgroup files (all four runners share one memory limit and one CPU weight;
# jferments/ci-infrastructure docs/mpfree-runner.md). Reads, never writes.
# The counters run from the slice's start, so a job prints them at its start
# and its end and the difference is that job's share. Never fails a job.
label=${1:-now}
s=/sys/fs/cgroup/system.slice/system-mpfree.slice
if [ ! -r "$s/memory.events" ]; then
  echo "slice report ($label): no mpfree slice here"
  exit 0
fi
echo "slice report ($label), $(date -u +%H:%M:%SZ)"
for f in memory.current memory.high memory.peak memory.events memory.pressure cpu.stat cpu.pressure; do
  [ -r "$s/$f" ] && sed "s/^/  $f: /" "$s/$f"
done
grep '^cpu ' /proc/stat | awk '{print "  vm steal ticks: " $9}'
exit 0
