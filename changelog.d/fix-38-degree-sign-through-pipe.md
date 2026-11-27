### Fixed

- The README's new Scripts section explains why `°C` shows as `┬░C` when PowerShell captures
  `sol`'s text output on a console's default code page. `sol` writes UTF-8, and PowerShell decodes
  it with the console's code page. Scripts should use `--json`, which carries temperatures as
  numbers, or set `[Console]::OutputEncoding = [Text.Encoding]::UTF8` before capturing text (#38)
