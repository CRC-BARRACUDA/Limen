# When something is wrong

## A module's tab says it is inactive

Two different things look the same here, and the tab says which:

- **It would not start** — a runtime that is missing, a library that would not
  load, a manifest it could not read. Usually something missing on this machine.
- **It panicked** — the module crashed part-way through a call and Limen caught
  it. The module is still loaded, but its state is whatever the crash left
  behind. Reopen the tab, and if it repeats, it is a bug in the module.

## A module is installed but not in the list

It is probably for another operating system — see **Installing and updating
modules**. `limen-cli list` reads the lockfile and shows everything installed,
including what this machine does not load.

## Nothing happens when a module asks the fleet

Modules that reach a fleet need credentials, and those live in one place — the
`crowdstrike` module — rather than in each module that uses them. Configure it
once. A host that is offline cannot be reached at all, which is why the host list
only offers the ones that are online.

## The window is too small, or the text is

**Settings** has the interface scale. The same page also turns animations off,
which is worth doing over a remote desktop session.

## Reading the log

The **Developer** tab carries Limen's own log, including every line the modules
report. It is the first place to look when something silently did not happen.
