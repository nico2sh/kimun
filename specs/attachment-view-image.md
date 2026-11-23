# Image in the attachment view

Date: 2026-11-23. Status: approved by the user.

## Intent

The attachment view shows what the editor area holds while an attachment is open. For a text file it shows a scrollable preview. For an image it shows only the header and "No preview". The view must draw the image, as the editor and the read-only views already do for embeds.

## Goals

- Opening a supported image shows it in the preview area of the attachment view.
- The image uses the shared image pieces, so it gets the cache, the EXIF rotation, the size limit, and the failure notes with no new decode code.

## Non-goals

- No zoom, no pan, no pixel size in the header.
- No tooltip, because the view shows no alt text.
- No change to the header, the "Open externally" button, the FollowLink chord, or the keys.
- No change for text files, or for files that are not supported images.
- No image in the terminal app.

## Anti-goals

- A scrollbar or scrolling for an image.
- An image drawn larger than its natural size.
- Text or controls that move when the pixels arrive.

## What the user decided

- Fit the whole preview area. The 60% height cap of the editor does not apply here. The image is drawn at natural size, never scaled up. It is scaled down only so that it fits the area's width and height. It sits centered.

## Approach

One sensible way. The view already picks a preview by file kind (`Preview::Text`, `Preview::None`). It gets one more kind, an image. The load task keeps the image's file path when the extension is a supported image format. The render draws the image with the shared loader. A second approach, drawing the image as rows of a list, does not fit, because this view needs no scrolling.

## How it works

- **Which files:** PNG, JPEG, GIF (first frame), WebP, and SVG. Any other file stays as today: "No preview" with "Open it externally to view it."
- **Size:** natural size, never scaled up, scaled down to fit the preview area's width and height with the aspect ratio kept, centered. The shared fit function gets a variant that takes a maximum width and a maximum height. The editor's rule (the column width and 60% of the pane height) becomes one call of that variant.
- **Loading:** the header gives the size at once. The decode runs in the background. An empty box of the fitted size shows until the pixels arrive. The image is pinned while the view shows it. A finished decode only redraws and requests nothing.
- **Failure:** an image that cannot be drawn keeps the "No preview" message, and its description is the failure note: "image not found", "image too large", or "cannot draw image".
- **Resize:** the fit is a pure function of the area's size, so the image fits again at each resize.
- **Unit:** sizes are in device pixels, as everywhere else. The painted size is the fitted size divided by the window scale factor.

## Constraints

- The GUI only reads files. Core owns vault path work. The view gets the real file path from `NoteVault::path_to_pathbuf`.
- The uncommitted or untracked files of other work are not part of this change.

## Left to the builder

- How the view measures its preview area.
- The names of the new items, within the plan's interfaces.
- The seam for counting decodes in tests, which exists in `ImageStore`.
