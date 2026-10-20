// Copyright header, then every directive that zenith reads.
module github.com/acme/server // the main module

go 1.23.0

toolchain go1.23.4

godebug default=go1.21
godebug (
	panicnil=1
	asynctimerchan=0 // keep the old timers
)

tool golang.org/x/tools/cmd/stringer
tool (
	github.com/a/gen
	"github.com/b/lint"
)

ignore ./node_modules
ignore (
	./testdata
)

require github.com/pkg/errors v0.9.1
require (
	golang.org/x/sync v0.8.0
	"gopkg.in/yaml.v3" v3.0.1 // indirect
	github.com/stretchr/testify v1.9.0 // indirect; test only
	github.com/davecgh/go-spew v1.1.1 // not indirect
)

replace github.com/pkg/errors => ../errors
replace github.com/old/lib v1.0.0 => github.com/new/lib v1.0.1
replace (
	golang.org/x/sync => ./third_party/sync
	github.com/b/c v2.0.0 => /opt/c
	github.com/win/lib => ..\win\lib
)

exclude golang.org/x/net v0.1.0
exclude (
	golang.org/x/text v0.3.0
)

retract v1.0.0 // published by mistake
retract [v1.1.0, v1.2.0]
retract (
	v1.3.0
	[v1.4.0, v1.5.0]
)

frobnicate everything
frobnicate (
	a b
	c d
)
