### The messages about the module tree and the import graph.

## Why a node could not be added to the graph.

graph_invalid_path = `{$path}` is not a normalised POSIX path relative to the project root
graph_invalid_name = `{$name}` is not a name of the form `<scheme>:<name>`
graph_kind_conflict = `{$name}` is already {$existing}, so it cannot also be {$added}
graph_too_many_nodes = the graph cannot hold more than {$max} nodes

## A kind of node, with its article, as the errors name it.

graph_kind_directory = a directory
graph_kind_file = a file
graph_kind_external = an external package
graph_kind_builtin = a builtin

## The overview of a module: its tree, with the number of files in each
## directory, and the dependencies between its children.

graph_overview_directory = {$name} ({$files ->
    [one] {$files} file
   *[other] {$files} files
})
graph_overview_dependencies = dependencies between the children of `{$module}`:
graph_overview_dependency = {$from} -> {$to} ({$imports ->
    [one] {$imports} import
   *[other] {$imports} imports
})

## How the dependencies between the nodes some levels below a module
## changed from one graph to another: between its children at depth 1.

graph_diff_nodes = {$depth ->
    [1] the children of `{$module}`
   *[other] the nodes {$depth} levels below `{$module}`
}
graph_diff_changed = dependencies between {graph_diff_nodes} that changed:
graph_diff_unchanged = no dependencies between {graph_diff_nodes} changed
