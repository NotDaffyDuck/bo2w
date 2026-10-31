# Contributing to bo2mp

bo2mp is a personal, experimental fan project. The code is open so people can read it, build it and
fix it; that does not oblige anyone to take on every request, and the maintainer makes the
decisions.

## Reporting a bug

You are welcome to report bugs, with no promise about when one is fixed. Reports are weighed
against what the project says it does:

> Something broken in a scenario the project says works matters more than a feature it never
> promised.

The claims are in `README.md`. "Do everything the original game did" is not a roadmap entry.

A report needs these and nothing else:

* the commit or release tag you ran
* your operating system and GPU
* the map you were on
* the steps you took and what happened instead of what you expected

Do not attach your `.env`, a memory dump or any archive of the game. Those are not read, and the
game archive cannot be accepted at all.

## Code changes

* **Small, standalone changes** (a fix, a crash, one wrong number, a docs typo): send them
  directly.
* **Design-level changes** (a new crate or subsystem, or a change in how data moves between `sim`,
  `render_frontend` and `net`): open an issue first. A big branch that shows up unannounced may be
  declined for reasons that have nothing to do with how good it is.

Before sending anything, run:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

`CONTEXT.md` describes how the maintainer works: artifacts, iterations and agent clones. That is a
private workflow, not a requirement. Changing the runtime needs none of it, nor anything under
`context/`.

## Original game data

Do not attach, commit or link any original game file (maps, weapons, sounds, executables). This
covers issues, pull requests and release assets alike. bo2mp reads data from a Black Ops II
installation that each user already owns, and the project is no place to distribute it.

## Who writes the code

An LLM writes all of this project.
