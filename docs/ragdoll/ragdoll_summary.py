"""Sums up a BO2MP_RAGDOLLLOG=1 run log: per body, its launch, top root speed, how far its first rigid body turned in
all, how far the root went, when it settled, and its worst joint pop (one-step move against the root) after its
first second, once it has landed. Bodies that never settled are listed too.
usage: python -I ragdoll_summary.py <run.log>
"""
import re
import statistics
import sys

START = re.compile(r"ragdoll log: #(\d+) starts at \(([-\d]+),([-\d]+),([-\d]+)\) launch (\d+) u/s, root speed (\d+) u/s")
TICK = re.compile(r"ragdoll log: #(\d+) t=([\d.]+) root .* speed (\d+) u/s turned (\d+) deg(?: jump ([\d.]+) u)?")
AGAIN = re.compile(r"ragdoll log: #(\d+) thrown again at \(([-\d]+),([-\d]+),([-\d]+)\) launch (\d+) u/s")
SETTLE = re.compile(r"ragdoll log: #(\d+) settles t=([\d.]+) turned (\d+) deg, root moved (\d+) u(?:, at \(([-\d]+),([-\d]+),([-\d]+)\))?")
ROOT = re.compile(r"root \(([-\d]+),([-\d]+),([-\d]+)\)")
# A body slides while its root lies within this of where it comes to rest.
SLIDE_HEIGHT = 12


def slide(path, rest, settled):
    """How far the root went across the ground (from the first tick at its resting height) and for how long."""
    low = [(t, x, y) for t, x, y, z in path if abs(z - rest[2]) <= SLIDE_HEIGHT]
    if not low:
        return 0.0, 0.0
    pts = [(x, y) for _, x, y in low] + [rest[:2]]
    far = sum(((b[0] - a[0]) ** 2 + (b[1] - a[1]) ** 2) ** 0.5 for a, b in zip(pts, pts[1:]))
    return far, settled - low[0][0]


def main(path):
    bodies = {}
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if m := START.search(line):
                bodies[m[1]] = dict(launch=int(m[5]), speed0=int(m[6]), top=int(m[6]), turned=0, settled=None, moved=None, jump=0.0)
            elif m := AGAIN.search(line):
                b = bodies.setdefault(m[1], dict(launch=0, speed0=0, top=0, turned=0, settled=None, moved=None, jump=0.0))
                b["again"] = b.get("again", 0) + 1
                b["settled"] = None
                b["launch"] = max(b["launch"], int(m[5]))
                b["path"] = []
                print(f"  #{m[1]} thrown again at ({m[2]},{m[3]},{m[4]}), {m[5]} u/s")
            elif (m := TICK.search(line)) and m[1] in bodies:
                b = bodies[m[1]]
                if r := ROOT.search(line):
                    b.setdefault("path", []).append((float(m[2]), *map(int, r.groups())))
                b["top"] = max(b["top"], int(m[3]))
                b["turned"] = int(m[4])
                if m[5] and float(m[2]) >= 1.0:
                    b["jump"] = max(b["jump"], float(m[5]))
            elif (m := SETTLE.search(line)) and m[1] in bodies:
                b = bodies[m[1]]
                b["settled"], b["turned"], b["moved"] = float(m[2]), int(m[3]), int(m[4])
                if m[5]:
                    b["slide"] = slide(b.get("path", []), tuple(map(int, m.groups()[4:7])), b["settled"])
    if not bodies:
        print("  no ragdoll log lines (was BO2MP_RAGDOLLLOG=1 set, and did anyone die?)")
        return
    print(f"  {len(bodies)} bodies")
    print("    #    launch  top u/s  turned deg  moved u  settled s  pop u")
    for k, b in bodies.items():
        settled = f"{b['settled']:.2f}" if b["settled"] is not None else "never"
        moved = b["moved"] if b["moved"] is not None else "-"
        print(f"    {k:>4} {b['launch']:>7} {b['top']:>8} {b['turned']:>11} {moved:>8} {settled:>10} {b['jump']:>6.1f}")
    print(f"  thrown again: {sum(b.get('again', 0) for b in bodies.values())} times, "
          f"{sum('again' in b for b in bodies.values())} bodies")
    done = [b for b in bodies.values() if b["settled"] is not None]
    if done:
        print(f"  median: turned {statistics.median(b['turned'] for b in done):.0f} deg, "
              f"moved {statistics.median(b['moved'] for b in done):.0f} u, settled {statistics.median(b['settled'] for b in done):.2f} s")
    print(f"  turned under 30 deg: {sum(b['turned'] < 30 for b in bodies.values())}; over 360 deg: "
          f"{sum(b['turned'] > 360 for b in bodies.values())}; never settled: {len(bodies) - len(done)}")
    worst = max(bodies.items(), key=lambda kb: kb[1]["jump"])
    print(f"  worst joint pop after landing: {worst[1]['jump']:.1f} u (#{worst[0]})")
    thrown = [(k, b) for k, b in bodies.items() if b["launch"] > 0 and "slide" in b]
    if thrown:
        k, b = max(thrown, key=lambda kb: kb[1]["slide"][0])
        print(f"  thrown bodies: {len(thrown)}; longest slide {b['slide'][0]:.0f} u (#{k}, launch {b['launch']} u/s), "
              f"{b['slide'][1]:.2f} s from touching down at its resting height to rest; "
              f"median slide {statistics.median(b['slide'][0] for _, b in thrown):.0f} u")


if __name__ == "__main__":
    main(sys.argv[1])
