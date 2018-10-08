# Socket collection

|Crate|Documentation|Linux/OS X|Windows|
|:---:|:-----------:|:--------:|-------|
| [![](http://meritbadge.herokuapp.com/socket-collection)](https://crates.io/crates/socket-collection) | [![Documentation](https://docs.rs/socket-collection/badge.svg)](https://docs.rs/socket-collection) | [![Build Status](https://travis-ci.org/ustulation/socket-collection.svg?branch=master)](https://travis-ci.org/ustulation/socket-collection) | [![Build status](https://ci.appveyor.com/api/projects/status/ajw6ab26p86jdac4/branch/master?svg=true)](https://ci.appveyor.com/project/MaidSafe-QA/socket-collection/branch/master)

The goal of this crate is to provide a collection of async sockets which can be used out of the box with `mio` event loop. As a simple example, using stream based protocols will require some sort of mechanism to determine the boundaries of a message etc., and this crate provides default implementation to handle those and abstract the boilerplate from the user libs.
