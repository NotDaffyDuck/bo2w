"""One hidden, muted, Idle-priority TDM match of a copied bo2mp.exe, <=180 s, then a summary of its log.

  hits: client 0 is the autoplay shooter (aims at the nearest enemy's chest and fires, stands still); 1 friendly and
        3 enemy bots fight normally. BO2MP_HITLOG logs every hit and every hit marker the scripts show, BO2MP_SNDLOG
        every script sound. Summary: docs/hits/hit_summary.py (per shooter, hits vs markers shown to him).
  hitsound: the hits match, drawn, so the client's impact sounds run too: BO2MP_SNDLOG also logs each head hit's
        crack (prj_bullet_impact_headshot). Same summary.
  bots: 3 v 3 bots at difficulty 2, client 0 idle. BO2MP_BOTLOG logs each bot stuck every second it makes no
        headway. Summary: docs/bots-stuck/stuck_summary.py (stretches of 3 s or more, worst first).
  ragdoll: like hits, so bodies drop. BO2MP_RAGDOLLLOG logs each body's root, speed and turn every 0.1 s and when it
        settles. Summary: docs/ragdoll/ragdoll_summary.py (per body: launch, top speed, turn, distance, settle time).
  killcam: 3 v 3 bots, client 0 idle, so he dies and watches killcams. BO2MP_KILLCAMLOG logs each replay frame's
        clock and every body that jumps. Summary: docs/killcam/killcam_summary.py (per replay: stutter, jumps).
  streaks: client 0 gets each scorestreak in turn through BO2's own givekillstreak (`give killstreak`), picks it with
        his streak key, fires, fires again (a location or a placement), waits. Bots stand still (BO2MP_BOTS_STILL).
        Two halves (--streaks a|b), 11 each, to stay under 180 s. Summary: docs/streaks/streak_summary.py (per streak:
        given, its INBOUND callout heard, script errors while it ran).
  hellstorm: like streaks, but drawn: client 0 flies one Hellstorm, lets the bomblets go, and a screenshot is taken
        every 0.5 s of the flight (bo2mp-artifacts/screenshots/hs_*.png) for its minimap marks. Summary:
        docs/streaks/hellstorm_summary.py (menu music off at the match start, the sound master's clipping, the
        Hellstorm view, the shots).
  blast: drawn, bots still: client 0 throws one frag ahead of him and a screenshot is taken every 0.5 s for 8 s
        after it goes off (bo2mp-artifacts/screenshots/bl_*.png), for what the explosion leaves on screen. Summary:
        docs/fx/blast_summary.py.
  Hitsound, bodies, killcams, the Hellstorm and the blast are drawn, so those run in a hidden window (BO2MP_WINDOW_HIDDEN=1) instead of headless.

One Python process does the waiting (no shell poll loop). Kills only what it started: its own PID, then any
bo2mp.exe whose path is this run folder's copy.
usage: python -I hidden_match.py hits|hitsound|bots|ragdoll|killcam|streaks|hellstorm|blast --bin <folder with bo2mp.exe> [--map mp_nuketown_2020] [--name NAME] [--streaks a|b] [--oldschool]
       [--gametype tdm|conf|...] (BO2MP_GAMETYPE; conf is Kill Confirmed, on TDM spawns and teams)
"""
import argparse
import os
import shutil
import subprocess
import sys
import time

GAMES = "C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"
CAP_S = 180
IDLE_PRIORITY_CLASS = 0x40
CREATE_NO_WINDOW = 0x08000000
DOCS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

KINDS = {
    "hits": (
        dict(BO2MP_BOTS="1,3,2", BO2MP_AUTOPLAY="1", BO2MP_HITLOG="1", BO2MP_SNDLOG="1"),
        os.path.join(DOCS, "hits", "hit_summary.py"),
        False,
    ),
    "hitsound": (
        dict(BO2MP_BOTS="1,3,2", BO2MP_AUTOPLAY="1", BO2MP_HITLOG="1", BO2MP_SNDLOG="1"),
        os.path.join(DOCS, "hits", "hit_summary.py"),
        True,
    ),
    "bots": (
        dict(BO2MP_BOTS="2,3,2", BO2MP_AUTOPLAY="0", BO2MP_BOTLOG="1"),
        os.path.join(DOCS, "bots-stuck", "stuck_summary.py"),
        False,
    ),
    "ragdoll": (
        dict(BO2MP_BOTS="1,3,2", BO2MP_AUTOPLAY="1", BO2MP_RAGDOLLLOG="1"),
        os.path.join(DOCS, "ragdoll", "ragdoll_summary.py"),
        True,
    ),
    "killcam": (
        dict(BO2MP_BOTS="2,3,2", BO2MP_AUTOPLAY="0", BO2MP_KILLCAMLOG="1"),
        os.path.join(DOCS, "killcam", "killcam_summary.py"),
        True,
    ),
    "streaks": (
        dict(BO2MP_BOTS="0,1,1", BO2MP_BOTS_STILL="1", BO2MP_AUTOPLAY="0", BO2MP_SNDLOG="1"),
        os.path.join(DOCS, "streaks", "streak_summary.py"),
        False,
    ),
    "hellstorm": (
        dict(BO2MP_BOTS="0,1,1", BO2MP_BOTS_STILL="1", BO2MP_AUTOPLAY="0", BO2MP_SNDLOG="1"),
        os.path.join(DOCS, "streaks", "hellstorm_summary.py"),
        True,
    ),
    "blast": (
        dict(BO2MP_BOTS="0,1,1", BO2MP_BOTS_STILL="1", BO2MP_AUTOPLAY="0"),
        os.path.join(DOCS, "fx", "blast_summary.py"),
        True,
    ),
}

# BO2's menu names for its scorestreaks (level.menureferenceforkillstreak), cheapest first. The VTOL gunner sits
# last in a: BO2 allows one "chopperInTheAir" per team (_killstreakrules.gsc), so after b's attack helicopter it is
# refused (KILLSTREAK_HELICOPTER_GUNNER_NOT_AVAILABLE), as in BO2.
STREAKS = {
    "a": ["spyplane", "rcbomb", "missile_drone", "supply_drop", "counteruav", "microwave_turret", "remote_missile",
          "planemortar", "auto_turret", "ai_tank_drop", "helicopter_player_gunner"],
    "b": ["helicopter_comlink", "spyplane_direction", "helicopter_guard", "emp", "straferun", "remote_mortar",
          "minigun", "dogs", "missile_swarm", "m32", "qrdrone"],
}


def kill_by_path(exe):
    ps = (
        "Get-CimInstance Win32_Process -Filter \"Name='bo2mp.exe'\" | "
        f"Where-Object {{ $_.ExecutablePath -eq '{exe}' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force; $_.ProcessId }}"
    )
    return subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout.split()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("kind", choices=KINDS)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--map", default="mp_nuketown_2020")
    ap.add_argument("--name")
    ap.add_argument("--streaks", choices=STREAKS, default="a")
    ap.add_argument("--gametype", default="tdm")
    # Oldschool's explosion push throws every body on death (level.oldschool, set at 15 s, before the first death).
    ap.add_argument("--oldschool", action="store_true")
    a = ap.parse_args()
    extra, summary, drawn = KINDS[a.kind]
    d = f"headless/{a.name or f'fan-{a.kind}{a.streaks if a.kind == "streaks" else ""}-{a.map}'}"
    os.makedirs(f"{d}/bo2mp-artifacts", exist_ok=True)
    for f in ("run.log", "run.out"):
        if os.path.exists(f"{d}/{f}"):
            os.remove(f"{d}/{f}")
    shutil.copy2(f"{a.bin}/bo2mp.exe", f"{d}/bo2mp.exe")
    exe = os.path.normpath(f"{d}/bo2mp.exe")
    log = os.path.normpath(f"{d}/run.log")

    env = dict(os.environ)
    for k in ("BO2MP_WINDOW_HIDDEN", "BO2MP_HEADLESS", "BO2MP_BOTS_STILL", "BO2ZM_LUI_PAUSE_AT", "BO2MP_MATCH_STEPS"):
        env.pop(k, None)
    env.update(
        BO2MP_GAMES=GAMES, BO2MP_SOUND="off", BO2MP_LOG=log, BO2MP_SCRIPTS="1",
        BO2MP_GAMETYPE=a.gametype, BO2MP_AUTOCLASS="custom1", BO2MP_SETTING_scorelimit="0", BO2MP_SETTING_timelimit="6",
        **extra,
    )
    env["BO2MP_WINDOW_HIDDEN" if drawn else "BO2MP_HEADLESS"] = "1"
    if a.oldschool:
        env["BO2MP_T6_SETLEVEL"] = "oldschool=1"
    cmds = "wait spawn; wait 140s; finish_run"
    if a.kind == "streaks":
        # A given streak lands in the inventory slot (BO2's care-package route): the switch key held 250 ms+ takes it up.
        # Use held 1.5 s then leaves a streak flown by hand (Reaper, Dragonfire, VTOL gunner); BO2 takes no other
        # streak while one is. The first waits out the pre-match countdown (15 s; frozen, the key does nothing).
        # Fire is held 1 s, past a marker's pin pull (holdFireTime 600 ms); three clicks because Lightning Strike takes
        # three locations (_planemortar.gsc).
        each = ("give killstreak killstreak_{0}; wait 1s; press +weapnext_inventory 0.4; wait 2s; press +attack 1.0; "
                "wait 1.5s; press +attack 0.3; wait 1s; press +attack 0.3; wait 3s; press +activate 1.5; wait 1.5s; ")
        cmds = "wait spawn; wait 15s; " + "".join(each.format(k) for k in STREAKS[a.streaks]) + "finish_run"
    if a.kind == "hellstorm":
        # The missile leaves about 2 s after the key; fire 1 s into the flight lets the bomblets go before it lands.
        shots = "".join(f"screenshot hs_{k:02}; wait 0.5s; " for k in range(2))
        later = "".join(f"screenshot hs_{k:02}; wait 0.5s; " for k in range(2, 16))
        cmds = ("wait spawn; wait 15s; give killstreak killstreak_remote_missile; wait 1s; "
                "press +weapnext_inventory 0.4; wait 1s; " + shots + "press +attack 0.3; " + later + "wait 3s; finish_run")
    if a.kind == "blast":
        # A frag cooks 3 s after the throw (BO2's fuse); shots from the throw to 8 s after the blast.
        shots = "".join(f"screenshot bl_{k:02}; wait 0.5s; " for k in range(22))
        cmds = "wait spawn; wait 16s; screenshot bl_pre; press +frag 0.2; " + shots + "finish_run"
    print(f"== {a.kind} on {a.map}: {exe}")
    t0 = time.monotonic()
    with open(f"{d}/run.out", "w") as out:
        p = subprocess.Popen([exe, "map", f"t6:{a.map}", "--cmds", cmds], cwd=d, env=env, stdout=out,
                             stderr=subprocess.STDOUT, creationflags=IDLE_PRIORITY_CLASS | CREATE_NO_WINDOW)
        try:
            code = p.wait(timeout=CAP_S)
        except subprocess.TimeoutExpired:
            print("  TIMED OUT at 180 s")
            subprocess.run(["taskkill", "/PID", str(p.pid), "/F"], capture_output=True)
            code = None
    leftovers = kill_by_path(exe)
    print(f"  ran {time.monotonic() - t0:.0f} s, exit code {code}; killed by path afterwards: {leftovers or 'none'}")
    if not os.path.exists(log):
        print("  FAIL: no log")
        return 1
    with open(log, encoding="utf-8", errors="replace") as f:
        panics = [l.rstrip() for l in f if "panicked" in l][:5]
    for l in panics:
        print(f"  PANIC: {l[:200]}")
    subprocess.run([sys.executable, "-I", summary, log])
    return 0


if __name__ == "__main__":
    sys.exit(main())
