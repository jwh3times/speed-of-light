### Fixed

- `sol status` and `sol watch` no longer end "can't reach the service" with
  `Caused by: The system cannot find the file specified. (os error 2)`, which read like a missing
  file. That error only means nothing is serving the pipe, which the message already explains. Any
  other error, such as access denied or the pipe being busy, is still shown as the cause (#39)
