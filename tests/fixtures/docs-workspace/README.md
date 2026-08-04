# Fixture Workspace

This directory is the workspace the end-to-end suite runs kmd against. It exists
so the tests assert on content this repository owns, rather than on whatever
markdown happened to be in the developer's working directory.

The token `KMDFIXTURE` appears only here and is used to prove search returns
highlighted results.

kmd renders documentation with a reactive frontend, so the word "reactive" is
searchable across more than one fixture file.

## Links

- [Architecture](guide/architecture.md)
