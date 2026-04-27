# Images

## Fetch Images

Takes a list of image URLs and retrieves their contents, creating an image list.

Images are automatically downsampled to avoid limits on request size imposed by
most providers.

> [!note]
> Vision models typically will scale down images with any dimension greater
> than about 500 pixels.

## Load Images

Loads images from local disk and stores scaled copies in the node data.
This increases the file size of the graph but prevents the workflow from
breaking when exported to other systems or if source URLs change.
