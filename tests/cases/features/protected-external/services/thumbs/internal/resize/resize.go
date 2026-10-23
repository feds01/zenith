package resize

import (
	"image"

	"github.com/disintegration/imaging"
)

func Thumbnail(img image.Image) image.Image {
	return imaging.Thumbnail(img, 256, 256, imaging.Lanczos)
}
