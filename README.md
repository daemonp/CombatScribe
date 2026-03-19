# CombatScribe

A fast, native desktop combat log parser and viewer for Turtle WoW. Load a `WoWCombatLog.txt` and get instant raid analysis — damage meters, healing meters, timelines, loot, and full ability breakdowns — without uploading anything to a website.

**Why local?** Your logs stay on your machine. No upload wait, no internet required, no privacy concerns. Parse a full raid night in under a second.

## Install

### Windows

Download **[combat-scribe-windows-x86_64.zip](https://github.com/daemonp/CombatScribe/releases/latest/download/combat-scribe-windows-x86_64.zip)**, extract, and double-click `combat-scribe.exe`.

### Linux

```sh
curl -sL https://github.com/daemonp/CombatScribe/releases/latest/download/combat-scribe-linux-x86_64.tar.zst | tar --zstd -xf - && chmod +x combat-scribe && ./combat-scribe
```

Or download from the [releases page](https://github.com/daemonp/CombatScribe/releases/latest) manually.

### macOS

A universal `.dmg` is available on the [releases page](https://github.com/daemonp/CombatScribe/releases/latest). Mount it and drag to Applications.

## Quick Start

1. **Load** — Click **Load File** (or drag a `.txt` / `.zip` onto the window) and pick your `WoWCombatLog.txt`.
2. **Pick a session** — CombatScribe auto-detects raid sessions (Molten Core, BWL, Naxx, etc.) and names them for you.
3. **Browse** — Switch between tabs to explore your raid.
4. **Export** — Click **Export** to save a cleaned-up log (`.txt` or `.zip`) ready for upload to MonkeyLogs / TurtLogs.

---

## Features at a Glance

| Tab | What you get |
|---|---|
| **Damage/Healing** | Side-by-side meters for the full raid. Damage done, damage done + pets, damage taken, effective healing, raw healing, overhealing. Click any player to drill down. |
| **Utility** | Dispels, interrupts, deaths, resurrections, absorbs, avoidance (dodge/parry/block), buff uptime, and consumable usage. |
| **Timeline** | Encounter timeline charting raid DPS, DTPS, HPS, deaths, big hits, and alive count. Aura waterfall with consumable/world buff presets. Zoomable. Event log with death replay. |
| **Loot** | Boss-grouped loot with WoW item quality colors. Search by item, player, or boss. Trade tracking included. |
| **Events** | Raw combat log browser, color-coded by event type, filterable by player. |

### Player Detail Overlay

Click any player on a meter to open a detailed breakdown:

- **Summary stats** — total, per-second, duration, hits, crits, crit rate
- **Opener sequence** — first 10 seconds of ability casts with timing gaps
- **Ability table** — every ability ranked by total damage/healing with hit count, crit%, average, and percentage
- **Damage taken** — grouped by attacker with full mitigation columns (absorb, resist, block, crush)
- **Class, race, guild, talent spec, gear count** shown in the header

### Encounter Detection

Automatic boss detection for all Turtle WoW content: Molten Core, BWL, AQ20, AQ40, ZG, Onyxia, Naxxramas, Lower & Upper Karazhan, Emerald Sanctum, Scarlet Citadel, and 8 dungeon instances. Kill/wipe tracking with per-boss attempt numbering.

Filter by **All Combat**, **All Kills**, **All Wipes**, **Trash**, or any individual encounter.

### Export & Formatting

The export pipeline cleans raw logs for upload: resolves "You/Your" to your character name, attributes pet and totem damage to owners, normalizes apostrophes, annotates self-damage, and fixes loot formatting. Optionally compresses to `.zip` and can zero the original log file after export.

**Batch export** — The UI supports exporting all raid sessions as individual dated zip files with one click. A CLI mode is also available:

```sh
combat-scribe --export WoWCombatLog.txt [output_dir] [--zero]
```

Each raid session is exported as `Player-Raid-YYYY-MM-DD-export.txt.zip`. Multiple sessions on the same day get collision-safe suffixes (`-2`, `-3`, etc.).

---

## Screenshots

### Damage and Healing Meters

Side-by-side damage and healing for an Upper Karazhan 40-player raid. Every player is ranked with class icon, class-colored name, total damage or healing, per-second throughput, and raid percentage. The encounter dropdown (top) lets you filter by boss, kills, wipes, or trash.

<img alt="Damage and healing meters showing a 40-player Upper Karazhan raid with ranked player bars, class icons, and per-second throughput" src="https://github.com/user-attachments/assets/b912166b-6a99-4f53-ba91-5df4e443bc3e" />

### Player Damage Breakdown

Clicking a player opens their full ability breakdown. This shows a Rogue's damage profile: opener sequence with cast timing, then a table of every ability sorted by total damage — Auto Attack, Backstab, Blade Flurry, Eviscerate — with hit counts, crit rates, and averages. Player info (class, race, guild, gear count) is shown at the top.

<img alt="Detailed damage breakdown for a Rogue showing opener sequence, ability table with hit counts, crit rates, and averages" src="https://github.com/user-attachments/assets/4c214449-3210-4869-89e0-21f301e64fb4" />

### Damage Taken Breakdown

A tank's damage taken view grouped by source. Each attacker (Mephistroth, Desolate Doomguard, Hellfire Imp, etc.) has its own ability table with full mitigation columns — absorb, resist, block, and crushing blows — so you can see exactly where incoming damage is coming from and how it's being mitigated.

<img alt="Tank damage taken breakdown grouped by attacker source with mitigation columns for absorb, resist, block, and crush" src="https://github.com/user-attachments/assets/b293aa76-e52a-4a87-8d96-c696ee30cf4a" />

### Encounter Timeline

A per-fight timeline showing raid DPS, damage taken, healing, deaths (red vertical lines), and big hits overlaid on a time axis. The alive count chart below tracks how many players are still standing. Click and drag to zoom into any time window. Toggle data series on/off from the legend. The event log below syncs to chart position for death replay analysis.

<img alt="Encounter timeline chart with raid DPS, HPS, DTPS, death markers, and alive count over a 1:34 boss fight" src="https://github.com/user-attachments/assets/2a885836-13d1-4fab-98d5-c91d90fcfc9e" />

### Consumes Tracking

<img width="2560" height="1600" alt="image" src="https://github.com/user-attachments/assets/570282be-93c1-4729-8eb7-311ea8dda70e" />

---

## Combat Log Addon

CombatScribe requires logs from **[SuperWowCombatLogger](https://github.com/pepopo978/SuperWowCombatLogger)**, which produces enhanced combat logs with class/talent info, pet attribution, and spell tracking that the default WoW log doesn't include. It requires the **[SuperWoW](https://github.com/balakethelock/SuperWoW)** client modification.

**Setup:**
1. Install [SuperWoW](https://github.com/balakethelock/SuperWoW)
2. Drop [SuperWowCombatLogger](https://github.com/pepopo978/SuperWowCombatLogger) into `Interface/AddOns/`
3. Remove `AdvancedVanillaCombatLog` if present
4. Enable in-game — logs go to your WoW `Logs/` folder

## Building from Source

```sh
cargo build --release
# Binary: target/release/combat-scribe
```

## License

BSD-2-Clause
