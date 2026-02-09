# iroh ping

A very simple iroh protocol for pinging a remote node. It's a high level example & easy starting point for new projects.

Walk through the quickstart example in the [Documentation Website](https://docs.iroh.computer/quickstart).

## Running the Examples

### Quickstart Example

This example demonstrates basic ping functionality between two endpoints.

First, start the receiver in one terminal:
```sh
cargo run --example quickstart receiver
```

The receiver will print a ticket. Copy this ticket, then in another terminal run:
```sh
cargo run --example quickstart sender <TICKET>
```

Replace `<TICKET>` with the ticket printed by the receiver.

## This is not the "real" ping

Iroh has all sorts of internal ping-type messages, this is a high level demo of a protocol, and in no way necessary for iroh's normal operation.

## License

Copyright 2026 N0, INC.

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
