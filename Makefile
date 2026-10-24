# Shortcuts for running the game and shipping it. Settings come from .env at
# the repo root (start from .env.example). Ship targets only call
# `cargo xtask`; run `cargo xtask` alone to see what it can do.
#
#   make map mp_raid                 load a map (CMDS='spawn 0; ...' runs console lines)
#   make map ZONE=t6:mp_raid         same, for names with a colon make cannot take as a goal
#   make play demo0000               play bo2mp-artifacts/demos/demo0000.bo2mpdemo
#   make menu                        the front end
#   make release prod|dev            build a release; deploy = release + publish

TOP := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
-include $(TOP)/.env
export

PROFILE ?= play
CMDS ?=
ZONE ?=
RELEASE ?=
SINCE ?=

# Words after the first goal are its arguments (`make map mp_raid`), so each
# one becomes a goal that does nothing.
VERB := $(firstword $(MAKECMDGOALS))
REST := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
ifneq ($(REST),)
.PHONY: $(REST)
$(REST): ; @true
endif

IN_TOP = cd "$(TOP)" &&
GAME = $(IN_TOP) cargo run --profile $(PROFILE) -p launcher --
XTASK = $(IN_TOP) cargo run --quiet -p xtask --
WITH_CMDS = $(if $(CMDS),--cmds '$(CMDS)')
WITH_ZONE = $(if $(ZONE),--zone $(ZONE))
MAP = $(or $(ZONE),$(firstword $(REST)))
CHANNEL = $(firstword $(REST))

.PHONY: help games map export-gltf play menu menu-shots \
	launcher build-windows setup-windows release publish deploy provision logs channel clean

help:
	@echo "game:  make map <zone> | export-gltf <zone> | play <demo> | menu | menu-shots"
	@echo "       CMDS='...' console lines, ZONE=t6:<map>, PROFILE=play|release|dev"
	@echo "ship:  make launcher windows | build-windows | setup-windows | provision"
	@echo "       make release|publish|deploy|logs prod|dev   (RELEASE=<id>, SINCE=2h)"
	@echo "disk:  make clean   removes the dev profile build"

games:
	@[ -n "$(BO2MP_GAMES)" ] || { echo "set BO2MP_GAMES in .env (see .env.example)"; exit 1; }

map: games
	@[ -n "$(MAP)" ] || { echo "make map <zone>, or ZONE=t6:<zone>"; exit 2; }
	$(GAME) map $(MAP) $(WITH_CMDS)

export-gltf: games
	@[ -n "$(MAP)" ] || { echo "make export-gltf <zone>"; exit 2; }
	$(GAME) export-gltf $(MAP)

play: games
	@[ -n "$(firstword $(REST))" ] || { echo "make play <demo name>"; exit 2; }
	$(GAME) play $(firstword $(REST)) $(WITH_ZONE) $(WITH_CMDS)

menu: games
	$(GAME) menu $(WITH_CMDS)

menu-shots: games
	mkdir -p "$(TOP)/bo2mp-artifacts/menu-shots"
	BO2MP_MENU_SHOT_DIR="$(TOP)/bo2mp-artifacts/menu-shots" $(GAME) menu

launcher:
	@[ "$(firstword $(REST))" = windows ] || { echo "make launcher windows"; exit 2; }
	PROFILE="$(PROFILE)" $(XTASK) release bundles

build-windows:
	PROFILE="$(PROFILE)" $(XTASK) windows build

setup-windows:
	$(XTASK) windows setup

provision:
	$(XTASK) provision

# release, publish, deploy and logs all take prod or dev.
channel:
	@case "$(CHANNEL)" in prod|dev) ;; *) echo "make $(VERB) prod|dev"; exit 2 ;; esac

release: channel
	PROFILE="$(PROFILE)" $(XTASK) release $(CHANNEL)

publish: channel
	RELEASE="$(RELEASE)" $(XTASK) publish $(CHANNEL)

deploy: channel
	PROFILE="$(PROFILE)" $(XTASK) release $(CHANNEL)
	RELEASE="$(RELEASE)" $(XTASK) publish $(CHANNEL)

logs: channel
	SINCE="$(SINCE)" $(XTASK) logs $(CHANNEL)

clean:
	$(IN_TOP) cargo clean --profile dev
