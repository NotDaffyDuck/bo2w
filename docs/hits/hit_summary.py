"""Summarise a BO2MP_HITLOG + BO2MP_SNDLOG run: per shooter, hits dealt vs hit markers shown to him."""
import collections, re, sys

hits = collections.Counter()       # shooter -> hits dealt to someone else
heads = collections.Counter()      # shooter -> head hits dealt
taken = collections.Counter()      # victim -> hits taken from someone else
shown = collections.Counter()      # client -> markers faded in for him
kinds = collections.Counter()      # marker picture faded in -> count
pictures = collections.Counter()   # marker picture names set
sounds = collections.Counter()     # hit sound alias -> count
hit = re.compile(r"bo2mp hit: player (\d+) by Some\((\d+)\) with (\S+) \((\S+)\): (\S+) for")
mark = re.compile(r"bo2mp hit marker shown: (\S+) from alpha \S+ over \S+ to Some\((\d+)\)")
pic = re.compile(r"bo2mp hit marker: (\S+) ")
snd = re.compile(r"bo2mp sndlog \S+ (mpl_hit\S*|\S*headshot\S*) (.*)")
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    if m := hit.search(line):
        victim, shooter, _, _, loc = m.groups()
        if victim != shooter:
            hits[shooter] += 1
            taken[victim] += 1
            heads[shooter] += loc in ("head", "helmet")
    elif m := mark.search(line):
        shown[m.group(2)] += 1
        kinds[m.group(1)] += 1
    elif m := pic.search(line):
        pictures[m.group(1)] += 1
    elif m := snd.search(line):
        sounds[m.group(1) + " -> " + m.group(2).strip()] += 1
print("client  hits_dealt  head_hits  hits_taken  markers_shown")
for c in sorted(set(hits) | set(taken) | set(shown), key=int):
    print(f"{c:>6}  {hits[c]:>10}  {heads[c]:>9}  {taken[c]:>10}  {shown[c]:>13}")
print("marker pictures:", dict(pictures))
print("markers shown by type:", dict(kinds))
stray = {c: n for c, n in shown.items() if hits[c] == 0}
print("markers shown to clients who hit nobody:", stray or "none")
for k, n in sounds.most_common(20):
    print(f"sound {n:>4}  {k}")
