package orders

import (
	"testing"

	"example.com/shop/internal/store" // zenith-ignore orders-not-store: tests may seed the store
)

func TestCount(t *testing.T) {
	if Count() != store.Size() {
		t.Fatal("count differs from the store")
	}
}
