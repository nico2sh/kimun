# Remote images

Date: 2026-11-23. Status: approved by the user on 2026-11-23 (the design and the plan). The builder check is done. The decisions are also in `adr/0063-remote-images-load-only-on-request.md`. My own calls are in the last section.

## Intent

The GUI draws vault images in place of `![alt](path)` and `![[name]]` lines. It does so in the editor, the facet view, the Preview pane, the Source reader, and the attachment view. A note can also embed an image with a web address, `![alt](https://host/path)`. Today that line stays plain styled text and the app makes no request.

This work lets a reader see remote images, without a request that the reader did not choose. Nothing loads until the reader asks, or until the host is on the reader's allowed list, as in an email client.

## Goals

- A remote embed line shows a placeholder that names the host, with the actions "Load" and "Always load from" that host. This holds in every surface that draws images.
- A loaded remote image draws as a vault image does: same size rule, same formats, same pins, same tooltip.
- A saved copy of each loaded image lives in an image cache outside the vault. A saved copy shows with no request, so a note shows its remote images offline.
- The reader sees and undoes every choice: the allowed hosts and the cache are in Preferences.

## Non-goals

- No remote SVG. The renderer is not yet checked for web and local file requests.
- No save to the vault as an attachment. It is a later feature and needs a naming rule.
- No change for the terminal app.
- No automatic refresh of a changed remote image. The reader reloads it.
- No `http` loading, no cookies, no login, and no proxy settings in this version.
- No remote image in the attachment view. That view opens vault files only.

## Anti-goals

- A request before a click, except to an allowed host.
- A request that goes to a host the reader did not allow, for example through a redirect.
- A dead address that repeats a request at every redraw or every note open.
- A text jump that the reader did not cause. The one allowed jump is the row change that follows the reader's load of an image.
- A bad or huge file that freezes the app or fills the disk.

## What the user decided

- Never load unless the reader asks for each image. The reader can allow whole hosts, as email clients do.
- A saved copy lives in a cache folder.
- An allowed host is an exact host name. A redirect to a host that is not allowed asks again.
- The read-only views have the click actions on the placeholder, and not only the editor. A loaded image in a read-only view keeps the tooltip and nothing else.
- The text shifts once after the reader loads an image.
- Failures: a one-line reason, a "Retry" action, no saved failure. `http` is shown as blocked. Remote SVG is out for now.
- The finder accepts balanced parentheses, an address in angle brackets, and a title. A remote embed needs no image extension. The first bytes decide the format.
- On a placeholder that stopped at a redirect, "Load" lets this one address follow the redirect to the named host for the session. The host does not join the allowed list, and "Retry" does not carry the permission over.
- Preferences gets a "Remote images" section with the allowed hosts and a "Clear cached images" button. The cache cap is 200 MB. There is no master switch.
- The editor menu on a loaded remote image has "Open in browser", "Copy image address", "Reload", and "Clear image cache" (this image only). Double-click and FollowLink do the same as "Open in browser". There is no save to the vault.

## Approaches

1. **One remote loader in the GUI, with the finder and the cache folder in core (chosen).** Core finds the embeds. Core also owns the cache folder, because core owns every file step. The GUI fetches, and it hands the bytes to core to save. A saved copy is a file like a vault image. The existing loader sizes it, decodes it, caches it in memory, and pins it. Only the fetch and the placeholder are new.
2. **Fetch inside core.** It puts all the logic in one place. It breaks ADR-0019, which keeps core free of network code. It was rejected.
3. **Download into the vault at the first load.** The image then becomes a vault attachment and needs no cache. It writes into the reader's vault and changes the file list without being asked. It was rejected.

## How it works

### The finder (core)

`image_embeds` keeps its current result for vault embeds. A second kind, the remote embed, is found for a whole line of the form `![alt](address)`. The address needs no image extension, because many image addresses have none. The format check on the first bytes rejects a file that is not an image. The address can hold balanced parentheses. It can be in angle brackets. A title in quotes can follow it. A query string is part of the address. An address over 2048 characters is not an embed. A wikilink with an address, `![[https://...]]`, is not one either. Zones that exclude embeds today (code, frontmatter, prose, list and quote lines) exclude remote embeds too.

A remote embed has the row, the alt text, the title, and the address. The scheme is `https` or `http`. An `http` embed is found, so a surface can show it as blocked.

### The remote state (GUI)

Each remote address is in one of these states in a shared store beside the image store:

- **Blocked.** The host is not allowed and no saved copy exists. The placeholder shows with its two actions.
- **Insecure.** The scheme is `http`. A note shows "Not loaded: http is not secure" with no action.
- **Loading.** A fetch is under way.
- **Saved.** A copy is in the cache. From here the existing loader treats the saved file as an image file, and the usual states (sized, ready, failed) follow.
- **Failed.** A one-line reason shows, with a "Retry" action. The state lives in memory for the session. No view asks again for this address until the reader retries.

The store checks the cache first. A saved copy wins over every other state. An address that the reader loaded once shows with no request, even after its host leaves the allowed list.

### Loading

The reader clicks "Load" or "Always load from". The second also adds the host to the allowed list. When a surface shows an embed on an allowed host, the embed starts its own fetch. A surface shows only the embeds on screen. A fetch for one address runs once, however many views show it. The number of fetches at one time has a limit. In an editor that edits, the embed on the line under the caret does not load on its own. It loads when the caret leaves the line, so a half typed address starts no request.

The fetch:

- Uses `https` only.
- Follows at most 3 redirects. A redirect to a host that is not allowed ends the fetch. The address then has the state Blocked, and the placeholder names the new host. On that placeholder, "Load" lets this one address follow the redirect to that host for the session. "Always load from" allows the new host.
- Stops after 15 seconds. A download of more than 20 MB also stops it.
- Decides the format from the first bytes. It accepts PNG, JPEG, GIF, and WebP. Any other format, including SVG, is a failure with "Not an image" or "Format not supported".
- Sends no cookies and no `Referer`, and a generic `User-Agent`.

A fetch never runs on the UI thread. When it ends, the surfaces that show the address redraw. As with vault images, the end of a fetch or a decode never starts another request.

### The image cache (core)

Core owns a folder under the app folder (`app_dir`). A saved copy has a name that comes from the address, and a size and time that the loader reads like any file. Core offers six steps. They save the bytes for an address and find the saved file for an address. They remove one copy and report the total size. They empty the cache. They remove the oldest copies until the total is under 200 MB. A save writes a temporary file and then renames it, so a copy is never half written. The cache is never in the vault, and the vault walker never sees it.

A copy that cannot be decoded, because the file is damaged, shows the usual failure note. "Clear cached images" in Preferences removes it.

### The placeholder

A placeholder takes two rows (two lines in the editor and the facet view, two rows in the preview). It shows the host name in muted text and the two actions as text buttons. The theme roles are those of the vault image note and placeholder. A button press does not move the caret in the editor, and it does not select a result in a preview.

When an image loads, its rows change from two to the image height, as for a vault image. The text below moves once. The height comes from the header of the saved file, so the count is right before the first frame of the picture.

A failed load, an `http` address, and a blocked redirect keep the two rows of the placeholder, so the text below does not move when a placeholder turns into one of them. They show the note text of the vault image and add the action buttons that fit: "Retry" for a failure, "Load" and "Always load from" for a blocked host.

Where the vault image follows Reveal, the remote image does too. In the editor, the cursor row shows the raw markdown and the image or placeholder shows below it. In the Preview pane and the Source reader, the rule about a line with a match is unchanged. A facet chunk never shows the raw line.

### The editor menu

A right-click on a loaded remote image opens a menu of four actions: "Open in browser", "Copy image address", "Reload", and "Clear image cache". The last one removes only this image's saved copy and returns the line to the placeholder. "Reload" fetches the image again. The old copy stays, and stays drawn, until the new bytes arrive and replace it. A failed reload keeps the old copy. "Open in browser" uses the system handler that web links already use. Double-click and FollowLink do the same. The vault entry menu does not appear for a remote image.

The facet view and the preview show a loaded image with a tooltip. The tooltip holds the alt text, or the title, or the host and file name when both are empty. These views have no menu.

### Preferences

The section "Remote images" lists the allowed hosts, each with a "Remove" button. It also has a "Clear cached images" button next to the cache size. The allowed list is a list of plain host names. It lives in the same file and store as the other Preferences. A change shows at once in all open views. Removing a host does not delete saved copies.

## Constraints

- Core owns every file step. The GUI reads files and fetches. It never writes a file.
- Core stays free of network code (ADR-0019).
- The terminal app does not change. A remote embed there is plain text as before.
- Vault image behavior does not change. Every existing image test passes unchanged.
- The first 100 embeds of a note or chunk are drawn. Remote embeds count in that 100.
- Tests do not use the network. The fetch has a seam that tests replace.
- The decode limit of 20 MB and the memory cache rules of vault images apply to saved copies.

## Left to the builder

- Which HTTP client the GUI uses, and how it runs off the UI thread. It must work on Windows, macOS, and Linux with no system TLS library.
- The names, the module split, and how the remote embed is added to the finder's result.
- How the saved file name comes from an address, as long as it is stable and safe on every OS.
- The limit on fetches at one time. My call is 4, and the loader can need another number.
- How the placeholder buttons are built, and the exact sizes and colors.
- Where the remote state store lives, and how it reaches the surfaces.

## Docs

The desktop app page gains a section on remote images: the placeholder, the two actions, the allowed hosts, the cache, and the Preferences section. The glossary entries "Remote image", "Allowed host", and "Image cache" are written. The entry "Inline image" changes where it says that a remote URL stays a styled line.

## Calls made by me

These are my calls. None of them changes what the user chose. Please check them.

1. **Request headers.** No cookies, no `Referer`, and a generic `User-Agent` that does not name the user, the system, or the version.
2. **Fetches at once.** At most 4 at the same time. A host allowed by the reader can still produce a burst in a note with many images, and this keeps the burst small. Only embeds on screen start a fetch.
3. **A saved copy beats the allowed list.** An address in the cache shows with no request after its host leaves the list. Showing a copy makes no request.
4. **No remote image in the attachment view.** That view opens vault files.
5. **The tooltip of a loaded remote image** shows the alt text, or the title, or the host and file name.
6. **Format errors.** A file with the wrong format is a failure, and it is not saved in the cache.
7. **Cache write.** A save writes a temporary file and renames it.
8. **A failure is not saved.** It lasts for the session in memory. "Retry" is the way to ask again.
9. **Address key.** The cache key comes from the address as written, with the fragment (`#...`), the angle brackets, and the title removed, and with the scheme and host in lower case. Two spellings of one image share one copy. Host matching for the allowed list ignores case and the port, and uses the host's ASCII form.
10. **Cache eviction.** It runs after each save. It removes the oldest copies by save time, and it never removes the copy just saved. Showing a copy does not touch the file. A copy that the loader holds in memory stays valid, because the memory cache does not read the file again.
11. **Time limits.** The 15 seconds apply to the connection and to each wait for data. A body that keeps sending data can run longer. A total limit of 60 seconds ends any fetch.
