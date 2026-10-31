"""Sums up a `hidden_match.py hellstorm` run log: whether the menu music was turned off as the match started (BO2 stops
it there), the sound master's worst clipping (samples over full scale before and after its limiter), the Hellstorm
view and its bomblets, and the screenshots taken.
usage: python -I hellstorm_summary.py <run.log>
"""
import os
import re
import sys

MASTER = re.compile(r"sound master: (\d+) samples, (\d+) over full scale before the limiter \(peak ([\d.]+)\), (\d+) after")
ERROR = re.compile(r"(?<![0-9] )script error|[1-9][0-9]* script error|panicked", re.I)


def main(path):
    music_off, music_lines, view, bomblets, errors = 0, [], False, 0, 0
    samples = over_in = over_out = 0
    peak = 0.0
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if "sound: menu music" in line:
                music_lines.append(line.strip())
                music_off += "menu music off" in line
            elif m := MASTER.search(line):
                samples += int(m[1])
                over_in += int(m[2])
                over_out += int(m[4])
                peak = max(peak, float(m[3]))
            elif "hellstorm view true" in line:
                view = True
            elif "its bomblets are out" in line:
                bomblets += 1
            elif ERROR.search(line):
                errors += 1
    print(f"  menu music lines: {len(music_lines)}, turned off {music_off} time(s)")
    for l in music_lines[:4]:
        print(f"    {l[:160]}")
    print(f"  sound master: {samples} samples, {over_in} over full scale before the limiter (peak {peak:.2f}), "
          f"{over_out} after")
    print(f"  hellstorm view: {view}; bomblets out: {bomblets}; script errors: {errors}")
    shots = os.path.join(os.path.dirname(path), "bo2mp-artifacts", "screenshots")
    names = sorted(n for n in os.listdir(shots) if n.startswith("hs_")) if os.path.isdir(shots) else []
    print(f"  screenshots: {len(names)} in {shots}")


if __name__ == "__main__":
    main(sys.argv[1])
