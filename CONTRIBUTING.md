# Helping out

Bug reports are the most useful thing. When something looks or behaves differently from the
official game, say what you did, what you saw and what the official game does; a screenshot
(F2 saves one) helps a lot. The launcher's **Sessions** page has each game window's log, and
copying the end of it into the report saves a round trip.

## Changing the code

```sh
cargo test --workspace
tools/reference/fetch.py                  # once: Mojang's server and client jars, and Java, under target/
python3 -I tools/e2e/run.py [names]       # scenarios on a real server, no window needed
python3 -I tools/e2e/zoo.py [groups]      # pictures to look at
```

A few rules the project keeps:

- **Facts come from the game, not from memory.** Packet layouts, numbers and formulas are read
  from the jar (`tools/jread.py CLASS [METHOD]`) or from Mojang's data generator. The wiki is a
  good guide and has been wrong before.
- **Nothing of Mojang's goes in the repository.** Textures, models, sounds and text are read at
  run time from the files Lattice downloads.
- **Check it on a real server.** A change to how the game behaves comes with a scenario in
  `tools/e2e/scenarios.py` that checks the result from the server's side where it can.
- **Tests use their own port and folders** (25568, `target/e2e`), never a world or server of
  yours.
- `crates/lattice-data` is generated; don't edit it by hand. `tools/new-version.py` rebuilds it.

Commit messages say what changed for someone playing, in plain words.

## License

By contributing you agree that your work is licensed under the same terms as the project:
MIT or Apache-2.0, at the user's option.
