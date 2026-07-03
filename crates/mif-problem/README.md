# mif-problem

RFC 9457 Problem Details envelopes for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Every error-producing crate in this workspace keeps its own `thiserror` error
enum (see this workspace's `CLAUDE.md`, "Why `thiserror` for Errors" — there is
no shared top-level error type). This crate supplies the shared *envelope
shape* those enums map into: implement [`ToProblem`] for an error enum to give
it a serializable `application/problem+json` representation alongside its
existing `Display` output, without merging enums together.

## License

MIT
