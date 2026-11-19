# A Kumikit core plugin in Rust

A provider that answers every message with the message itself: the smallest complete
core plugin. Copy this folder to start your own.

## What you need

- Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`.
- The `kumikit` command (it packs, signs and installs plugins).

## Make it yours

1. In `package/plugin.toml`, set `id` to `<your publisher name>.<plugin name>`, and the
   name, description and repository.
2. Write your provider in `src/lib.rs`. If it starts a program, list the program in
   `[permissions] spawn`: users see "can start the `x` program, which runs with your full
   user access" and grant it before anything runs.
3. Make a publisher key once, outside any repository: `kumikit plugin keygen --name
   yourname`. It prints your public key; keep the private key file secret and backed up.

## Build, sign and try it

```sh
KUMIKIT_KEY=~/.config/kumikit/keys/yourname.key ./build.sh
kumikit plugin add yourname.echo-0.1.0.kkp
```

`kumikit plugin add` shows who signed the package and what it may do, then asks. Start a
thread with the Echo provider in the app.

## Publish it

Upload the `.kkp` (for example as a GitHub release asset) and open a pull request on the
registry with `publishers/<you>.toml` (your public key) and `plugins/<you>/<name>.toml`
(see [docs/spec/10-distribution.md](../../docs/spec/10-distribution.md#registry)).
Kumikit's interfaces are 0.x until 1.0: the official registry accepts early publishers
only until then.
