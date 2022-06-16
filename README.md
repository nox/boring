# boring

[![crates.io](https://img.shields.io/crates/v/boring.svg)](https://crates.io/crates/boring)

BoringSSL bindings for the Rust programming language and TLS adapters for [tokio](https://github.com/tokio-rs/tokio)
and [hyper](https://github.com/hyperium/hyper) built on top of it.

[Documentation](https://docs.rs/boring).

## Release Support

By default, the crate statically links with the latest BoringSSL master branch.

## Support for pre-built binaries

While this crate can build BoringSSL on its own, you may want to provide pre-built binaries instead.
To do so, specify the environment variable `BORING_BSSL_PATH` with the path to the binaries.

You can also provide specific headers by setting `BORING_BSSL_INCLUDE_PATH`.

_Notes_: The crate will look for headers in the `$BORING_BSSL_INCLUDE_PATH/openssl/` folder, make sure to place your headers there.

_Warning_: When providing a different version of BoringSSL make sure to use a compatible one, the crate relies on the presence of certain functions.

## Building with a FIPS-validated module

Only BoringCrypto module version ae223d6138807a13006342edfeef32e813246b39, as
certified with [certificate
3678](https://csrc.nist.gov/projects/cryptographic-module-validation-program/certificate/3678)
is supported by this crate. Support is enabled by this crate's `fips` feature.

`boring-sys` comes with a test that FIPS is enabled/disabled depending on the feature flag. You can run it as follows:
```bash
$ cargo test --features fips fips::is_enabled
```

## FIPS Frankenbuild

In order to have access to newer boringssl features while keeping FIPS compliancy,
there is the option to do a frankenbuild that links in the FIPS module (bcm.o) from the certified commit
against a newer build of boringssl.

Use the `frankenfips` feature to enable this and follow the below instructions to build a consuming lib/bin:

1. Install prebuilt FIPS-certified module (`apt-get install libbssl-fips-dev`, 
   see https://wiki.cfops.it/display/FEDRAMP/libbssl-fips+Implementation+Guide for background info)
2. Add this to the consumer's Cargo.toml:
```
[target.'cfg(feature = "frankenfips")']
rustflags = ["-C", "link-args=-Wl,-zmuldefs"]
```

(The libcrypto.a contains two versions of bcm.o files, we need to instruct ld to not error on multiple definitions.
Normally when a symbol is defined multiple times, the linker will report a fatal error.
Using "-z muldefs" ld allows multiple definitions and the first definition will be used.)


## RPK support

This is an internal cloudflare fork of the external https://github.com/cloudflare/boring repository.
The changes aren't upstreamed because google's version of boringssl doesn't have RPK support
("Raw Public Keys"; see https://datatracker.ietf.org/doc/html/rfc7250).

Note that RPK support and FIPS support are mutually incompatible.


## Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed under the terms of both the Apache License,
Version 2.0 and the MIT license without any additional terms or conditions.

## Accolades

The project is based on a fork of [rust-openssl](https://github.com/sfackler/rust-openssl).
