# Product roadmap

This document records future product direction separately from the shipped MVP. Confirmed items are commitments; proposed items still require product and implementation decisions. Unknowns remain explicitly unverified.

## Confirmed

### User-configurable interface language

Cook will provide an account-level interface language setting. The initial supported locales and fallback order are **unverified** and have not been selected yet. Implementation should avoid hard-coding a first locale set before that decision is made.

## Proposed

### Recipe image uploads

Recipe image upload is a post-MVP feature. A reasonable initial scope is one cover image per recipe, without galleries or inline instruction images.

The implementation complexity is moderate when images are stored on a local persistent volume, but it adds backup size, restore time, capacity monitoring, and orphan-file cleanup to operations. Object storage is likely easier to scale later. The final storage backend is **unverified** and remains undecided.

Any image-upload design must include these security and privacy controls:

- accept only JPEG, PNG, and WebP; reject SVG and other active or unsupported formats;
- validate by actually decoding the file server-side, then re-encode it into an approved format;
- remove EXIF and other metadata during re-encoding;
- enforce limits on uploaded bytes, decoded pixel dimensions, and images per recipe;
- generate random, non-guessable storage identifiers instead of trusting original file names;
- require authentication, recipe-owner authorization, storage quotas, and upload rate limits;
- never trust the client-provided file name, extension, or MIME type as validation;
- ensure images attached to private recipes cannot be fetched through a public or predictable URL;
- include image data in backup and restore procedures, and periodically remove orphaned files safely.

Before implementation, the storage choice, serving authorization model, size limits, image transformations, retention rules, and migration path must be documented and tested.
