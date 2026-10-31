"""Sums up a BO2MP_KILLCAMLOG=1 run log: per replay, how long it ran, how many frames it drew, how often its replay
clock stood still or went back (stutter), its biggest clock step, and the bodies that jumped during it.
usage: python -I killcam_summary.py <run.log>
"""
import re
import sys

BEGIN = re.compile(r"killcamlog: replay begins clock=(-?\d+)")
END = re.compile(r"killcamlog: replay ends clock=(-?\d+)")
FRAME = re.compile(r"killcamlog: frame=(\d+) replay_t=(-?\d+) effective=(\S+) body_t=(-?\d+)")
JUMP = re.compile(r"killcamlog: jump frame=(\d+) ent=(\d+) client=(-?\d+) e_type=(\S+) d=([\d.]+)")


def main(path):
    replays, cur = [], None
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if m := BEGIN.search(line):
                cur = dict(begin=int(m[1]), end=None, frames=0, still=0, back=0, step=0, last=None, lag=0, jumps=[])
                replays.append(cur)
            elif cur is None:
                continue
            elif m := END.search(line):
                cur["end"] = int(m[1])
                cur = None
            elif m := FRAME.search(line):
                t, body = int(m[2]), int(m[4])
                cur["frames"] += 1
                cur["lag"] = max(cur["lag"], abs(t - body))
                if cur["last"] is not None:
                    d = t - cur["last"]
                    cur["still"] += d == 0
                    cur["back"] += d < 0
                    cur["step"] = max(cur["step"], d)
                cur["last"] = t
            elif m := JUMP.search(line):
                cur["jumps"].append((int(m[3]), float(m[5])))
    if not replays:
        print("  no killcam replays in the log (did client 0 die, and was BO2MP_KILLCAMLOG=1 set?)")
        return
    print(f"  {len(replays)} replays")
    print("    #  clock ms  frames  stood still  went back  top step ms  body lag ms  jumps (client:d)")
    for i, r in enumerate(replays, 1):
        span = r["end"] - r["begin"] if r["end"] is not None else "open"
        jumps = " ".join(f"{c}:{d:.0f}" for c, d in r["jumps"][:6]) or "-"
        print(f"    {i:>2} {span:>8} {r['frames']:>7} {r['still']:>12} {r['back']:>10} {r['step']:>12} {r['lag']:>12}  {jumps}")


if __name__ == "__main__":
    main(sys.argv[1])
