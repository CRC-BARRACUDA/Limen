# Installing and updating modules

Open **Modules**. The page has two halves: what is installed, and what is
available from the module repositories.

## Installing

Find the module and press **Install**. Limen fetches it, resolves anything it
depends on, and installs those too. A module that needs a runtime it cannot find
— Python, say — will have one fetched for it into the Limen folder, so the
machine you are on stays as you found it.

The card shows a `#tag` or two. Clicking a tag filters the list by it, and the
search box matches names, descriptions and tags together.

## Updating

A module installed from a repository is checked for newer releases. When there
is one, the card offers **Update**, and the nav bar shows an
**Update available** pill for Limen itself.

## Removing

**Remove** deletes the module's folder, including anything it had fetched into
its own `tools/`. It asks first, because that is not a stray-click action.

## Why a module you installed might not be listed

A module can declare which operating systems it is for. One that is not for this
machine is not loaded at all — a module that reads the Windows licensing store
has nothing to say on Linux, and an empty list would read as "nothing is
licensed" rather than "wrong machine".
