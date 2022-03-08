This directory contains a dummy example to force cargo to include the "lock" file in the published
crate. That way, we can do a wasm build with the locked dependencies.

Unfortunately, simply "including" it doesn't work. We need to either add an example or a binary target.
