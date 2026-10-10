# Folder tree icons

Generated with built-in image_gen, transparent classic Windows Mail style.
Source: source-atlas.png. Runtime: ../../icons/folder-tree.png (384x384 RGBA).
Three columns, three rows, 128px runtime tiles. Row-major indices:
0 account mailbox; 1 local folders; 2 Inbox; 3 Outbox; 4 Sent;
5 Drafts; 6 Trash; 7 Junk; 8 generic/archive/recovered folder.

Prompt: Create a production desktop mail folder icon atlas PNG with transparent background, exact square canvas and exact 3 columns by 3 rows equal square cells, nine icons centered with 15% padding. Classic Windows Vista / Windows Mail small toolbar icon style, crisp silhouettes readable at 16px, restrained glossy yellow folders, silver trays, sky blue arrows, thin dark edges. No text, labels, numbers or grid lines. Fixed row-major order: mailbox/server with envelope; overlapping golden folders; inbox tray with envelope and blue down arrow; outbox tray with envelope and blue up arrow; sent folder with envelope and blue right arrow; drafts folder with paper and blue pencil; silver wastebasket with blue recycling mark; junk folder with envelope and red prohibition badge; generic golden folder with envelope. True transparent alpha.

Source resized to runtime atlas using System.Drawing HighQualityBicubic with SourceCopy, preserving alpha. The application embeds this PNG separately from the unchanged message-status atlas, resizes each tile independently for DPI, and uses the mailbox role for server folders.
