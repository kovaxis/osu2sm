

# Planned changes

- Use difficulties from `osu!.db`.
- Use `.ssc` instead of `.sm`.
- Convert standard, and perhaps taiko and ctb.
- Cache mp3 audio length query.
- Abort parsing quickly if osu! gamemode or keycount is not compatible.
- Optimize transform system to reuse buffers.
- Review whether it's useful to have each bucket consist of multiple lists, and if it is, implement
    it.
- Consider making osu! loader and simfile writer themselves a transform, and remove global `~in`
    and `~out`.
