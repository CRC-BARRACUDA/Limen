# Language

Limen speaks English and Ukrainian. Change it in **Settings**; the whole
interface follows immediately, including tabs already open.

## What gets translated

- **Limen's own chrome** — the nav, the pages, the dialogs.
- **Module cards** — a module can ship `locales/<code>.toml` beside its manifest
  with a translated title and description.
- **A module's own screens** — the host tells each module which language is
  active, and modules that carry catalogs answer in it.

A module with no translation for the active language shows its English text
rather than blanks. Nothing is hidden because it is untranslated.
