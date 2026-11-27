# Message transform API

Core WASM plugins export `workshop_abi_version() -> i32` returning 1, `workshop_buffer() -> i32` returning a buffer address, and `workshop_on_message(event: i32, length: i32) -> i32`.

Event 0 is an outgoing message, 1 is a received MESSAGE_CREATE, and 2 is a received MESSAGE_UPDATE. The host writes UTF-8 message content into the plugin buffer, calls the handler, and reads its returned number of UTF-8 bytes at the same address. The buffer must accommodate 65536 bytes. Output is limited to 65536 bytes. Each callback currently gets a new instance, 16 MiB maximum memory, and 10 million execution fuel units. Globals do not persist across messages. There are no WASI imports.

A trap, invalid pointer, invalid UTF-8 or other error preserves the original message. Legacy `workshop_transform(length)` modules remain supported. The active plugin comes from WORKSHOP_PLUGIN or the bundled Clear URLs module. The plugin enable switch applies to message callbacks.

This API transforms messages the user sends through Phoenix. It does not provide an API for plugins to send unsolicited messages. Outgoing interception applies to POST channel-message JSON requests. Received transformations affect the local event before Phoenix processes it; they do not edit messages on Discord's servers. Multipart uploads are not covered.

The native byte-buffer ownership ABI and transport locators were informed by the MIT-licensed Sunbird source, pinned in Cargo.lock. Workshop uses its own narrow message adapter and WASM event API.

# General plugin events

Plugins may additionally export `workshop_event(length: i32) -> i32`. The host writes an event JSON document at the address returned by `workshop_buffer()` and reads the returned JSON document at the same address. General event instances persist across calls. These plugins currently use the general event API rather than the separate text-transform callback.

Event types:

- `settings`: returns an array of boolean settings with `key`, `title`, `description`, and `default`. The host renders these under the plugin's card and persists their values by plugin ID.
- `initialize`: loads local plugin state before background work starts.
- `tick`: runs every second on a background worker while enabled. Returns `null`.
- `http_response`: receives `method`, API `path`, `status`, and parsed JSON `body`. Returns `null` to preserve it, or `{status, body}` to replace it. This is a local response transformation.
- `http_request_cleanup`: receives `method`, `path`, and JSON `body`; returns `{body}` to remove local synthetic identifiers before a save. This cleanup also runs after a plugin is disabled.
- `image`: receives `url` for a badge-icon image. Returns `null`, or a replacement HTTPS URL. The native adapter bounds the lifetime cache to 4096 distinct rewritten URLs so futures can borrow them safely.

The guest imports `workshop.call(request_ptr, request_length, response_ptr, response_capacity) -> response_length`. Requests and results are JSON. A negative result means failure.

Host operations:

- `time`: Unix seconds.
- `image_alias` with `source` and prepared `target`: registers a badge-icon URL mapping so image fetches can resolve without entering a busy guest. Mappings are bounded and only active while the plugin is enabled.
- `current_user`: the current account ID as `{id}`, or null if unknown. Desktop resolves the selected account from local preferences or a single unambiguous account store, then falls back to observed self responses; no credentials or private account fields.
- `settings`: the current plugin's saved boolean settings.
- `storage_get` with `key`: reads a plugin-scoped JSON value or null.
- `storage_set` with `key` and `value`: atomically stores a JSON value. Keys cannot contain path separators.
- `http_get` with `url`: returns `{pending:true}` while a bounded background job runs. Poll with the same URL to retrieve `{status, body}`. Only HTTPS hosts listed in the plugin manifest are permitted. Requests are unauthenticated, do not follow redirects, have a twenty-second timeout, and are limited to 8 MiB. Network requests are only allowed during background ticks.

General events are capped at 8 MiB and linear memory at 128 MiB. Foreground callbacks have 200 million fuel units; background ticks have 5 billion to parse bulk data. A busy plugin is skipped rather than blocking Phoenix's foreground callback. Traps discard the VM so a partially executed guest cannot retain a stuck mutex. Its disk storage survives. Failed callbacks preserve the original response or image URL.

The `plugin-event <id> '<event JSON>'` command supports individual events or an array dispatched sequentially in the same instance. It is a development diagnostic and does not require a running Discord process.

## Prepared images

The generic `image_url` host operation accepts `url`, `format` (`png`, `gif`, or `mp4`), `width`, and `height`. Hosts are controlled by the plugin manifest's `image_hosts` capability. It returns a URL on a randomly authenticated, loopback-only cache service. The image rewrite adapter accepts these host-owned URLs in addition to HTTPS URLs.

Downloads and conversions occur on image-service workers, not the Phoenix UI thread. The service pins public DNS addresses, refuses literal/private origins, follows no redirects, caps downloads at 8 MiB, and bounds conversion time and memory. PNG output uses the first frame; GIF output preserves animation up to 120 frames. Requested dimensions cannot exceed 128px. Cached files survive restarts; registrations and authentication tokens do not. ImageMagick, curl, and timeout must be available on Linux.
