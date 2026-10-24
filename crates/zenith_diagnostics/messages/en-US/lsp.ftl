### The messages of the language server of `zenith lsp`: what it logs, and
### what it answers the editor.

## What is logged.

lsp_published = published {NUMBER($diagnostics)} {$diagnostics ->
    [one] diagnostic
   *[other] diagnostics
} on {NUMBER($files)} {$files ->
    [one] file
   *[other] files
}

## What the editor is answered, and shown.

lsp_unknown_request = `zenith lsp` does not handle `{$method}` requests
lsp_unlocated = {NUMBER($count)} {$count ->
    [one] finding has
   *[other] findings have
} no place in a file to show {$count ->
    [one] it
   *[other] them
}; run `zenith check` to see {$count ->
    [one] it
   *[other] them
}
lsp_check_panicked = zenith crashed while checking the project: {$reason}; the server's log has the details
lsp_unknown_panic = no reason was given

## Why the server stopped talking to the editor, which `zenith lsp` fails with.

lsp_error_protocol = the editor broke the Language Server Protocol: {$error}
lsp_error_message = a message to or from the editor is invalid: {$error}
lsp_error_disconnected = the editor went away before it told the server to exit
lsp_error_io = could not talk to the editor over stdin and stdout: {$error}
