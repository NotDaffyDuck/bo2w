# Reporting a security problem

Please report through the **Report a vulnerability** button on the repository's Security tab. That
opens a private advisory which only the maintainer can read, and it is the right place whenever
the report contains details someone could use to attack a player.

This is a way to reach the maintainer, not a program. There is no bounty, no response deadline and
no promised fix date; one person reads the reports.

## In scope

bo2mp is an experimental runtime. Its network exists for playtests that people have agreed to join.
A report is most useful when the problem reaches beyond that:

* running code, or writing files outside `bo2mp-artifacts/`, because of map data, a replay, a demo
  file or another client's packets
* anything that changes, patches or removes files in a Black Ops II installation. bo2mp opens
  those files for reading and for nothing else
* the update path: any way for something other than the release host to hand the launcher a
  binary it will accept
* sending the machine's files, credentials or private keys to another player or to the master
  server

## Known and not a finding

* The wire protocol is not versioned between builds and changes freely, so two different releases
  failing to connect is expected.
* The master server passes traffic between clients and vouches for none of them. Joining a match
  means trusting the people in it, as `README.md` says.
* The runtime parses game data that the local user chose to point it at. If your own install's
  malformed data crashes it, that is a bug but not a vulnerability.

## What to include

Give the commit or release, the operating system, the data that reproduces it and the steps. A
crash backtrace or the file that triggers it beats a description. Never attach `.env`, private
keys or any game files.
