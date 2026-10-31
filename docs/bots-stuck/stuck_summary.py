"""Summarise a BO2MP_BOTLOG run: each bot's stretches stuck 3 s or more, longest first, with where."""
import re, sys

stuck = re.compile(r"bo2mp bot (\d+) stuck (\d+) ms at (-?\d+) (-?\d+) (-?\d+) .*?ahead_blocked (\w+) in_solid (\w+) touching (.*?) goal")
worst = {}  # (bot, rounded spot) -> (longest ms, ahead_blocked, in_solid, touching)
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    if m := stuck.search(line):
        bot, ms, x, y, z, blocked, solid, touching = m.groups()
        key = (bot, round(int(x), -1), round(int(y), -1), round(int(z), -1))
        if int(ms) > worst.get(key, (0,))[0]:
            worst[key] = (int(ms), blocked, solid, touching)
rows = sorted(((v[0], k, v) for k, v in worst.items() if v[0] >= 3000), reverse=True)
print(f"{len(rows)} stuck stretches of 3 s or more")
for ms, (bot, x, y, z), (_, blocked, solid, touching) in rows[:30]:
    print(f"bot {bot:>2}  {ms/1000:5.1f} s  at {x} {y} {z}  ahead_blocked {blocked}  in_solid {solid}  touching {touching}")
