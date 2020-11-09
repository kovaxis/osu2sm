

# Planned changes

- Use difficulties from `osu!.db`.
- Use `.ssc` instead of `.sm`.
- Perhaps convert taiko and ctb.
- Abort parsing quickly if osu! gamemode or keycount is not compatible.
- Use `bumpalo` for fastness.
- Optimize `get_each` and add a `take_each`, to take advantage of the fact that simfiles are
    already stored linearly by default.
- Apply text transformations to difficulty names.
- Add modes to `Remap`, specifically a mode that takes input notes just as an intensity reference
    (for better `osu!std` mapping).
