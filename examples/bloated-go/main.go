package main

import "fmt"

func main() {
	s := NewStore()
	s.Add("tea", 350, 2)
	s.Add("mug", 800, 1)
	fmt.Println("total:", s.Total())
}
