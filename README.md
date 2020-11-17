
# osu2sm

`osu2sm` is batch converter from Osu!-beatmaps to StepMania-simfiles, with support for various
features and map transformations.

Advanced configuration is possible, but a default configuration is provided.

# Getting started

Download the latest binaries and follow the following video:

[![osu2sm demo](https://img.youtube.com/vi/l1YLVsiuXZ8/0.jpg)](https://www.youtube.com/watch?v=l1YLVsiuXZ8)

# In-place conversion

`osu2sm` can convert your beatmaps in-place, without copying any images or `.mp3` files.
You can then create a link to your `osu!` song folder inside your `StepMania` song folder, or let
`osu2sm` do it automatically for you.
However, on Windows creating folder links sadly requires admin permissions, so you will have to run
`osu2sm` as administrator for this to work.

# Osu!standard beatmaps

There is experimental osu!standard beatmap conversion, but it is disabled by default.
To enable, set the `OsuLoad -> standard -> keycount` field in the config to something other than
`0`.
See the configuration file examples for more info.

# Configuration file

The converter is heavily configurable, with a node-based setup where each node takes and input and
creates an output.
Currently there is no documentation on the config format other than the source itself (under 
`src/node`).

There are some example configuration files under `examples`, which for example enable the
`osu!standard` converter.

As a simple starting point, the `input` field near the start of the configuration file can be
set to the path of the `osu!` song folder, and the `output` field near the end of the configuration
file can be set to the path of the `StepMania` song folder to automate the selection of song
folders.
