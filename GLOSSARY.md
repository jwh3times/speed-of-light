# Speed of Light

The language of a GPU tuning, fan control and monitoring tool whose promise is that every change
it makes is checked, proven safe for the driver, and fails towards cooling.

## Processes and parts

**Service**:
The one elevated process that owns the GPUs and makes every write. Every client goes through it.
_Avoid_: daemon, server, agent

**Client**:
Any program that talks to the service: the `sol` command, a future GUI, a script.
_Avoid_: frontend, app

**Backend**:
The thin translation layer between the service and one vendor's API.
_Avoid_: driver, plugin, provider

## Fans

**Firmware control**:
The card's own built-in fan behaviour, with Speed of Light not driving the fans. Profiles and
the command line spell it `auto`; everywhere else it is firmware control.
_Avoid_: auto (in prose), default fans, stock fans

**Startup takeover**:
The service putting every fan under firmware control the moment it starts, before anything
else, in case an earlier instance died while driving them.

## Profiles and safety

**Profile**:
A named set of power, clock and fan settings to apply to a GPU.
_Avoid_: preset, config

**Startup profile**:
The one profile the service applies when it starts. For the installed service it is an
administrator-owned copy taken at install time, never the user's own file; a console session
reads the file it was given.
_Avoid_: boot profile, default profile

**Service config**:
The administrator-owned settings the installed service runs with: its policy caps, whether it
allows unverified writes, how often it ticks, its stability window and how much it logs. Like
the startup profile, it is a copy taken at install time, never the user's own file; a console
session reads the file it was given.
_Avoid_: settings file, config (on its own)

**Policy caps**:
The user's own limits on power and clock offsets, set inside the range the driver allows. They
bind every write, whichever client asks, and no request can loosen them.
_Avoid_: user limits, max settings

**Stock**:
The card's default power limit, zero clock offsets and firmware control.
_Avoid_: default, factory settings

**Crash guard**:
The record that a profile changing power or clocks is still on trial. If it is still present
when the service next starts, that profile is not applied again.
_Avoid_: sentinel, safe mode

**Stability window**:
How long a profile must run before the crash guard counts it as having survived.

**Graceful stop**:
The service ending because it was asked to (a stop request, system shutdown, Ctrl+C): fans go
back to firmware control and the crash guard is cleared. Power and clock settings stay applied.
_Avoid_: shutdown (on its own), exit

**Killed**:
The service ending without a graceful stop (a crash, Task Manager). Nothing is handed back until
the service restarts and its startup takeover runs.
_Avoid_: crash (for the service; a crash is the PC's)

**Uninstall**:
Removing the service and returning every GPU to stock.

## Compatibility

**Compatibility matrix**:
The evidence-backed record of which vendor, feature, driver and architecture combinations are
verified or broken.
_Avoid_: support list, whitelist

**Verified**, **Unverified**, **Broken**:
A combination with evidence that writes work; one with no evidence either way (blocked unless
the user opts in); one with evidence that writes fail (always blocked).

**Not enforced**:
A write the driver accepted but that did not take effect when read back.
_Avoid_: silently ignored, applied (for such a write)
