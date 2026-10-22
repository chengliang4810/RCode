# Streaming selection and prompt expansion

Text selection releases automatic viewport following at mouse-down and
materializes the current list position. A stream burst must not remove the drag
anchor before the first mouse move. Selection retains the own-send runway;
edge scrolling and returning to the live tail use the existing navigation paths.

Show more adds the prompt's revealed height to the reservation using the prompt
resize curve. Show less reverses it. Expanding a prompt does not count as
assistant output or consume the reservation; new assistant output does.

## Validation

```sh
cargo test --locked -p rcode-ui --lib -- --test-threads=1
```

The UI tests exercise cached transcript views and mouse events, including a
burst between mouse-down and the first drag movement, selection across streaming
and completion, prompt expansion/collapse and returning to the tail. Related
coverage includes runway geometry, wheel escape, resizing, completion shrinkage
and viewport restoration.

For native acceptance, stream a reply while selecting text and expanding a long
prompt, then return to the tail. Check that the selection anchor remains stable
and new output consumes the reservation. Repeat on each supported desktop
platform; headless interaction tests do not establish native rendering quality.
