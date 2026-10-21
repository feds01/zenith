# Suppressions

A `zenith-ignore` comment exempts the imports on a line from the contracts it
names:

```ts
// zenith-ignore web-layers: legacy, tracked in ENG-123
import { x } from "@/domains/orders";

import { y } from "@/app/(dashboard)/types"; // zenith-ignore web-layers
```

- It works the same in Go files, where it is a `//` or `/* */` comment among the
  imports.
- The syntax is `zenith-ignore <id> [<id>…][: <reason>]`, in a line or a block
  comment. Ids may also be separated by commas.
- A comment after code applies to the imports that start on its own line. A
  comment on a line of its own applies to the imports on the next line.
- It removes those imports from the contracts it names, and no others.

A suppression that has no effect is a warning: one that names no contract,
names a contract that doesn't exist, or whose contracts, among those checked,
have no import on its line.
