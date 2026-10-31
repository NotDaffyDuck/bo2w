# Driving two local players through the master

The old `make duo` recipe is no longer in the Makefile. What remains are the
pieces that let any launcher script do the same job: console commands over
stdin, a master-state file, and a join-by-ID command. All of them read
`BO2MP_GAMES` and the `BO2MP_MASTER_*` settings from `.env`, as `make menu` does,
and the configured master may be local or remote.

## Host and join by hand

```bash
BO2MP_MASTER_HOST_NAME='name' make map mp_raid     # host opens a room
make menu                                          # the other player joins from the browser
```

`BO2MP_MASTER_HOST_NAME` and `BO2MP_MASTER_JOIN` are alternatives; setting both
is an error, and `BO2MP_MASTER_ADDR` needs one of them.

## Hooks for scripts

* `BO2MP_CONSOLE_STDIN=1` makes the game read console commands line by line from
  standard input. Each line goes into the normal console queue, so semicolons
  separate commands and `!` keeps its usual meaning of skipping a wait.
* `BO2MP_MASTER_STATUS_FILE=<absolute path>` makes the game write the current
  master state as `key=value` lines (`state`, `room`, `members`, `in_match`),
  replacing the file atomically.
* `ui_join_lobby_id <hex id>` sends the ordinary master join request. Set
  `ui_mapname` and `ui_gametype` to the lobby's choice first. Admission remains
  the master's decision.

None of these bypass networking or GSC. GSC still owns classes, countdown,
scoring, death and respawn.

## Commands worth sending

```text
spawn 0
wait world
dump host
screenshot
press +attack 0.5
quit
```

The retired runner kept each game's logs, settings, dumps and screenshots in
separate `host/` and `client/` directories; a script that starts two instances
should likewise keep their output apart (`BO2MP_SETTINGS_PATH` moves the
settings file).
