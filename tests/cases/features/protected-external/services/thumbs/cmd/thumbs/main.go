package main

import (
	"fmt"

	"github.com/disintegration/imaging"

	"example.com/thumbs/internal/resize"
)

func main() {
	img, err := imaging.Open("in.png")
	if err != nil {
		panic(err)
	}
	fmt.Println(resize.Thumbnail(img).Bounds())
}
