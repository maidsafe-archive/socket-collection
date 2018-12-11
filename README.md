# Socket collection

|Crate|Documentation|Linux/OS X/Windows|
|:---:|:-----------:|:-----------------:|
| [![](http://meritbadge.herokuapp.com/socket-collection)](https://crates.io/crates/socket-collection) | [![Documentation](https://docs.rs/socket-collection/badge.svg)](https://docs.rs/socket-collection) | [![Build Status](https://travis-ci.org/maidsafe/socket-collection.svg?branch=master)](https://travis-ci.org/maidsafe/socket-collection)

The goal of this crate is to provide a collection of async sockets which can be
used out of the box with `mio` event loop. As a simple example, using stream
based protocols will require some sort of mechanism to determine the boundaries
of a message etc., and this crate provides default implementation to handle
those and abstract the boilerplate from the user libs.
In addition, `socket-collection` supports optional encryption based on
[safe_crypto](https://github.com/maidsafe/safe_crypto) crate.

## License

socket-collection library is dual-licensed under the Modified BSD (
[LICENSE-BSD](https://opensource.org/licenses/BSD-3-Clause)) or the MIT license
( [LICENSE-MIT](http://opensource.org/licenses/MIT)) at your option.
