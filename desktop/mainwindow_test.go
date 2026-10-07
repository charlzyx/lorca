package main

import (
	"math"
	"testing"

	"github.com/egoist/mygo"
)

func TestWidened(t *testing.T) {
	area := mygo.Rectangle{X: 0, Y: 40, Width: 1920, Height: 1000}
	rect := func(x, width int) mygo.Rectangle { return mygo.Rectangle{X: x, Y: 100, Width: width, Height: 760} }
	tests := []struct {
		name string
		r    mygo.Rectangle
		by   int
		want mygo.Rectangle
	}{
		{"with room", rect(200, 900), 281, rect(200, 1181)},
		{"at the right edge", rect(1020, 900), 281, rect(739, 1181)},
		{"near the right edge", rect(920, 900), 281, rect(739, 1181)},
		{"at the left edge", rect(0, 900), 261, rect(0, 1161)},
		{"wider than the room", rect(100, 1700), 281, rect(0, 1920)},
		{"filling the area", rect(0, 1920), 281, rect(0, 1920)},
		{"partly off the display", rect(1500, 900), 281, rect(1219, 1181)},
		{"off both sides", rect(-100, 2200), 261, rect(-100, 2200)},
		{"all the room", rect(300, 900), math.MaxInt32, rect(0, 1920)},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := widened(test.r, area, test.by); got != test.want {
				t.Errorf("widened(%v, %d) = %v, want %v", test.r, test.by, got, test.want)
			}
		})
	}
}
