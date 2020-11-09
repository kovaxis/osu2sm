

# Planned changes

- Use difficulties from `osu!.db`.
- Use `.ssc` instead of `.sm`.
- Convert standard, and perhaps taiko and ctb.
- Cache mp3 audio length query.
- Abort parsing quickly if osu! gamemode or keycount is not compatible.
- Use `bumpalo` for fastness.
- Optimize `get_each` and add a `take_each`, to take advantage of the fact that simfiles are
    already stored linearly by default.
- Take amount of simultaneous notes into account for `EffectiveBpm`.
