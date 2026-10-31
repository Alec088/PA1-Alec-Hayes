#!/bin/bash
# Runs the scheduler against every sample test and checks:
#   * output vs. tests/*.out (exact diff; small timestamp jitter is allowed by
#     the real grader, so inspect any diff by eye)
#   * no zombie / leftover workload processes in the process table afterward
#   * error messages for the malformed inputs
# Usage (inside the container): bash scripts/run_tests.sh
set -u
cd "$(dirname "$0")/.."
cargo build -q 2>&1 || exit 1
export WORKLOAD_PATH=./target/debug/workload
SCHED=./target/debug/scheduler-gpt
WORK=/tmp/pa1-run
rm -rf "$WORK" && mkdir -p "$WORK"

# Lists every zombie (state Z) or workload process currently in /proc.
leftovers() {
    for s in /proc/[0-9]*/stat; do
        read -r line < "$s" 2>/dev/null || continue
        # stat format: pid (comm) state ...
        local comm=${line#*(}; comm=${comm%%)*}
        local state=${line##*) }; state=${state%% *}
        if [ "$state" = "Z" ] || [ "$comm" = "workload" ]; then
            echo "    pid ${line%% *} ($comm) state $state"
        fi
    done
}

for t in fcfs sjf rr unfinished; do
    cp "tests/$t.in" "$WORK/"
    $SCHED "$WORK/$t.in"
    echo "== $t (exit $?)"
    if [ -f "tests/$t.out" ]; then
        diff --strip-trailing-cr "tests/$t.out" "$WORK/$t.out" > /dev/null && echo "  output: MATCH" \
            || { echo "  output: DIFF"; diff --strip-trailing-cr "tests/$t.out" "$WORK/$t.out" | sed 's/^/    /'; }
    else
        sed 's/^/    /' "$WORK/$t.out"
    fi
    z=$(leftovers)
    [ -z "$z" ] && echo "  process table: clean" || { echo "  process table: LEFTOVERS"; echo "$z"; }

    # Orphaned zombies get reaped by the container's PID 1 once the scheduler
    # exits, so the process-table check above can't see a missing reap.
    # Instead, compare children created vs. children the scheduler itself
    # reaped (wait4 returning a PID) in a syscall trace.
    strace -f -qq -o "$WORK/$t.trace" -e trace=clone,wait4 -v $SCHED "$WORK/$t.in"
    # (strace may split a call into "<unfinished ...>" / "resumed>" lines;
    # only the line carrying the return value ends in "= <pid>".)
    forks=$(grep -E 'clone' "$WORK/$t.trace" | grep -cE '= [1-9][0-9]*$')
    reaps=$(grep -E 'wait4' "$WORK/$t.trace" | grep -E 'WIFEXITED|WIFSIGNALED' | grep -cE '= [1-9][0-9]*$')
    [ "$forks" = "$reaps" ] && echo "  reaped: $reaps/$forks children" \
        || echo "  reaped: ONLY $reaps/$forks children (zombie left behind)"
done

echo "== error handling"
$SCHED 2>&1; echo "  (exit $?)"
for t in missing_processcount missing_quantum; do
    cp "tests/$t.in" "$WORK/"
    $SCHED "$WORK/$t.in" 2>&1; echo "  (exit $?)"
done
