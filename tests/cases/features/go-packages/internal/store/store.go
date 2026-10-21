package store

import (
	"net/http"
	"os"
)

func Size() int {
	return len(os.Args) + http.StatusOK
}
