"""Sums up a `hidden_match.py blast` run log: the frag thrown, script errors, and the screenshots taken after it.
usage: python -I blast_summary.py <run.log>
"""
import os
import re
import sys

ERROR = re.compile(r"(?<![0-9] )script error|[1-9][0-9]* script error|panicked", re.I)


def main(path):
    errors, fx = 0, []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if ERROR.search(line):
                errors += 1
            elif "fx:" in line and "explo" in line.lower():
                fx.append(line.strip())
    print(f"  script errors: {errors}; explosion effect lines: {len(fx)}")
    for l in fx[:3]:
        print(f"    {l[:160]}")
    shots = os.path.join(os.path.dirname(path), "bo2mp-artifacts", "screenshots")
    names = sorted(n for n in os.listdir(shots) if n.startswith("bl_")) if os.path.isdir(shots) else []
    print(f"  screenshots: {len(names)} in {shots}")


if __name__ == "__main__":
    main(sys.argv[1])
