### The messages about JavaScript and TypeScript: their workspace packages,
### and the resolver of their imports.

## The workspace packages, which `package.json` `workspaces` and
## `pnpm-workspace.yaml` declare.

js_invalid_workspace_pattern = ignoring the invalid workspace pattern `{$pattern}`: {$error}
js_duplicate_package = the packages in `{$first}` and `{$second}` are both named `{$name}`
js_read = could not read `{$path}`: {$error}
js_parse = could not parse `{$path}`: {$error}

## Why the resolver can't be set up as `[languages.js]` says.

js_tsconfig_not_found = the tsconfig file `{$path}` does not exist
js_invalid_extension = `{$extension}` is not an extension, such as `.ts`
