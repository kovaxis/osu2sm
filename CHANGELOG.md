

# Planned changes

- Use difficulties from `osu!.db`.
- Use `.ssc` instead of `.sm`.
- Convert standard, and perhaps taiko and ctb.
- Cache mp3 audio length query.
- Abort parsing quickly if osu! gamemode or keycount is not compatible.
- Consider making osu! loader and simfile writer themselves a transform, and remove global `~in`
    and `~out`.
- Use `bumpalo` for fastness.
- Add true snap.
- Set difficulty number from in-practice BPM.
- Add `Chain` bucket.
- Optimize `get_each` and add a `take_each`.
