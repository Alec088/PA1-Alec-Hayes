#!/bin/bash
# Stress checks for the three concurrency bugs (run inside the container):
#   1. spawn/stop race   -- examples/spawn_race under heavy CPU load
#   2. missing reap      -- covered by scripts/run_tests.sh (unfinished.in)
#   3. EINTR in waitpid  -- strace fault injection makes every other wait4
#                           fail with EINTR, which a single unchecked
#                           waitpid call mishandles
# Usage: bash scripts/verify_fixes.sh [runs-per-test]
set -u
cd "$(dirname "$0")/.."
RUNS=${1:-3}
cargo build -q --bins --examples 2>&1 | grep -E '^error' -A6
export WORKLOAD_PATH=./target/debug/workload
SCHED=./target/debug/scheduler-gpt
mkdir -p /tmp/pa1-verify

echo "--- EINTR injection ($RUNS runs per test)"
for t in fcfs sjf rr; do
    for r in $(seq "$RUNS"); do
        cp "tests/$t.in" /tmp/pa1-verify/
        strace -f -qq -o /tmp/pa1-verify/trace -e trace=clone,wait4 \
            -e inject=wait4:error=EINTR:when=2+2 $SCHED "/tmp/pa1-verify/$t.in"
        diff -q --strip-trailing-cr "tests/$t.out" "/tmp/pa1-verify/$t.out" > /dev/null \
            && m=match || m=DIFF
        inj=$(grep -c INJECTED /tmp/pa1-verify/trace)
        forks=$(grep clone /tmp/pa1-verify/trace | grep -cE '= [1-9][0-9]*$')
        reaps=$(grep wait4 /tmp/pa1-verify/trace | grep -E 'WIFEXITED|WIFSIGNALED' \
            | grep -cE '= [1-9][0-9]*$')
        echo "$t run $r: output $m, $inj EINTRs injected, $reaps/$forks children reaped"
    done
done

echo "--- spawn race probe under load (40 busy loops)"
pids=()
for i in $(seq 40); do yes > /dev/null & pids+=($!); done
for r in $(seq "$RUNS"); do ./target/debug/examples/spawn_race 1000; done
kill "${pids[@]}"
