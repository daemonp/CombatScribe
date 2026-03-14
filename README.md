# wow-parse-rs

A native desktop application for parsing, formatting, and analyzing World of Warcraft 1.12 (Turtle WoW) combat logs. Built in Rust with the [iced](https://github.com/iced-rs/iced) GUI framework.

## Features

- **Log Export** -- Load a `WoWCombatLog.txt`, detect raid sessions, format the log for upload (player name resolution, pet attribution, apostrophe normalization, self-damage annotation, loot fixes), and export as a cleaned-up `.txt` or `.zip` file suitable for upload to log analysis sites like MonkeyLogs / TurtLogs.
- **Log Viewer** -- Parse combat logs into structured data and browse an interactive analysis UI with:
  - Damage and healing meters with per-player ability breakdowns
  - Encounter detection with kill/wipe tracking and per-boss attempt numbering
  - Utility tracking: dispels, interrupts, deaths, resurrections, absorbs, avoidance, buffs
  - Loot tables grouped by boss with item quality colors and trade tracking
  - Filterable event log

## Combat Log Generation

This parser expects combat logs produced by the [SuperWowCombatLogger](https://github.com/pepopo978/SuperWowCombatLogger) addon. Standard WoW combat logs are missing critical information that this parser depends on. **You must use SuperWowCombatLogger to generate your logs.**

### What SuperWowCombatLogger does

SuperWowCombatLogger is a WoW addon that produces enhanced combat logs with additional data not present in the default combat log. It requires the [SuperWoW](https://github.com/balakethelock/SuperWoW) client modification to function. Key enhancements over the default combat log include:

- **Zone and combatant metadata** -- Writes `ZONE_INFO` and `COMBATANT_INFO` lines containing the current instance, player names, classes, talents, and GUIDs, which the parser uses for session detection, roster building, and class-colored UI display.
- **Pet and totem attribution** -- Rewrites pet autoattacks as "Auto Attack (pet)" under the owner and attributes shaman totem damage, Greater Feral Spirit, Battle Chicken, and Arcanite Dragonling to their respective owners.
- **Self-damage separation** -- Annotates damage a player deals to themselves (e.g. Power Overwhelming) as `(self damage)` so it can be excluded from regular damage meters.
- **Missing spell tracking** -- Logs caster and target for spells the default combat log omits entirely: Faerie Fire, Sunder Armor, Curse of the Elements/Recklessness/Shadow/Weakness/Tongues, Expose Armor, and HoT cast events (Rejuvenation, Regrowth, Renew).
- **Buff/debuff stack counts** -- Adds initial stack counts `(1)` to buff/debuff gain messages so parsers can track stacking correctly.
- **No helper addon requirement** -- Unlike its predecessor (AdvancedVanillaCombatLog), no other raiders need to run a companion addon.

### Installation

1. Install [SuperWoW](https://github.com/balakethelock/SuperWoW) (client-side modification, required).
2. Clone or download [SuperWowCombatLogger](https://github.com/pepopo978/SuperWowCombatLogger) into your `Interface/AddOns/` directory so the folder structure is:
   ```
   Interface/AddOns/SuperWowCombatLogger/
     SuperWowCombatLogger.toc
     core.lua
     RPLLCollector.xml
     ...
   ```
3. Remove `AdvancedVanillaCombatLog` and `AdvancedVanillaCombatLog_Helper` from your addons folder if they exist.
4. Enable the addon in-game. Combat logging will produce an enhanced `WoWCombatLog.txt` in your WoW `Logs/` directory.

## Building

```sh
cargo build --release
```

The binary is produced at `target/release/wow-parse-rs`.

## Usage

Run the application:

```sh
cargo run --release
```

1. Click **Open Log File** and select a `WoWCombatLog.txt`.
2. The application detects raid sessions in the log. Select the session(s) you want to work with.
3. **Export** -- Format and export the log for upload to MonkeyLogs / TurtLogs.
4. **View** -- Open the interactive log viewer to browse damage meters, healing, utility, loot, and events.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `iced` | GUI framework |
| `rfd` | Native file dialog |
| `regex` | Log line pattern matching |
| `zip` | ZIP archive creation for export |
| `chrono` | Timestamp formatting |
| `tokio` | Async runtime |

## License

This project is not currently published under a formal license.
