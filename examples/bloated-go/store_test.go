package main

import "testing"

func TestTotal(t *testing.T) {
	s := NewStore()
	s.Add("tea", 350, 2)
	s.Add("mug", 800, 1)
	if got := s.Total(); got != 1500 {
		t.Fatalf("total = %d", got)
	}
}

func TestEmpty(t *testing.T) {
	if got := NewStore().Total(); got != 0 {
		t.Fatalf("total = %d", got)
	}
}
