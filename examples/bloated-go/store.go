package main

import (
	"sort"
	"strings"
)

type Item struct {
	Name     string
	Cents    int
	Quantity int
}

type Store struct {
	items []Item
}

func NewStore() *Store {
	return &Store{}
}

func (s *Store) Add(name string, cents, quantity int) {
	s.items = append(s.items, Item{Name: name, Cents: cents, Quantity: quantity})
}

func (s *Store) Total() int {
	total := 0
	for _, item := range s.items {
		total += item.Cents * item.Quantity
	}
	return total
}

// names lists the item names, upper-cased and sorted. The old CLI printed it.
func (s *Store) names() []string {
	out := make([]string, 0, len(s.items))
	for _, item := range s.items {
		out = append(out, strings.ToUpper(item.Name))
	}
	sort.Strings(out)
	return out
}

type legacyRow struct {
	Label string
	Value int
}

func (r legacyRow) text() string {
	return strings.Repeat("-", r.Value) + r.Label
}

func legacyRows(s *Store) []legacyRow {
	rows := []legacyRow{}
	for _, item := range s.items {
		rows = append(rows, legacyRow{Label: item.Name, Value: item.Quantity})
	}
	return rows
}

const legacyWidth = 80
