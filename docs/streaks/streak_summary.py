"""Sums up a `hidden_match.py streaks` run log: per scorestreak, whether BO2's givekillstreak took it, whether its
INBOUND callout played (KILLSTREAK_X_INBOUND or the strafe run's MP_WAR_STRAFERUN_INBOUND), whether its owner heard
the "used" voice (vox_*_kls_*_used: BO2 plays it once the streak is in use, INBOUND or not; a placed turret plays
neither in BO2, so its mpl_turret_startup counts, shown as "placed"), whether the streak key was
pressed while the player was still frozen (pre-match), and the script errors logged while it was the latest one given.
usage: python -I streak_summary.py <run.log>
"""
import re
import sys

GIVE = re.compile(r"bo2mp streaks: give player 0 killstreak_(\w+) \(started (\w+)\)")
MISSING = re.compile(r"bo2mp streaks: give killstreak_(\w+): no such scorestreak")
CALLOUT = re.compile(r"player_callout\|(?:KILLSTREAK|MP_WAR)_(\w+?)_INBOUND")
USED = re.compile(r"sndlog local vox_\w+?_kls_(\w+)_used Client\(SeatNo\(0\)\)")
PLACED = re.compile(r"sndlog play mpl_turret_startup ")
FROZEN = re.compile(r"bo2mp inventory: switch asked;.* frozen true")
ERROR = re.compile(r"SCRIPT ERROR|script error|panicked", re.I)


def main(path):
    rows, cur, callouts = {}, None, []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if m := GIVE.search(line):
                cur = m[1]
                rows[cur] = dict(started=m[2] == "true", heard=[], used=[], frozen=False, errors=[])
            elif m := MISSING.search(line):
                rows[m[1]] = dict(started=False, heard=[], used=[], frozen=False, errors=["no such scorestreak"])
            elif cur and (m := USED.search(line)):
                rows[cur]["used"].append(m[1].lower())
            elif cur and "turret" in cur and PLACED.search(line):
                rows[cur]["used"].append("placed")
            elif cur and FROZEN.search(line):
                rows[cur]["frozen"] = True
            elif m := CALLOUT.search(line):
                callouts.append(m[1].lower())
                if cur:
                    rows[cur]["heard"].append(m[1].lower())
            elif cur and ERROR.search(line) and " 0 script errors" not in line:
                rows[cur]["errors"].append(re.sub(r" @0x[0-9a-f]*:", ":", line.strip())[:160])
    if not rows:
        print("  no streaks given (did client 0 spawn? is `give killstreak` in this build?)")
        return
    print("    streak                     given  callout heard          used voice         errors")
    for k, r in rows.items():
        heard = ",".join(sorted(set(r["heard"]))) or "-"
        used = ",".join(sorted(set(r["used"]))) or "-"
        frozen = "  (key pressed while frozen)" if r["frozen"] else ""
        print(f"    {k:<26} {'yes' if r['started'] else 'NO':<6} {heard:<22} {used:<18} {len(r['errors'])}{frozen}")
        for e in r["errors"][:2]:
            print(f"        {e}")
    called = sum(bool(r["heard"] or r["used"]) for r in rows.values())
    print(f"  {len(rows)} given, {called} called in (INBOUND, used voice or turret placed), {sum(bool(r['errors']) for r in rows.values())} with errors")


if __name__ == "__main__":
    main(sys.argv[1])
