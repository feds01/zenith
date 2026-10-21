package orders

import (
	"github.com/pkg/errors"

	"example.com/shop/internal/store"
)

func Count() int {
	return store.Size()
}

func Fail() error {
	return errors.New("no orders")
}
