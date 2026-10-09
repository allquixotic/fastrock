package ui

import (
	"fmt"
	"runtime"
	"strings"
	"unicode"
)

func splitSettingWords(input string) ([]string, error) {
	return splitWordsForPlatform(input, runtime.GOOS == "windows")
}
func splitWordsForPlatform(input string, windows bool) ([]string, error) {
	var words []string
	var word strings.Builder
	inWord := false
	quote := rune(0)
	chars := []rune(input)
	for i := 0; i < len(chars); i++ {
		c := chars[i]
		if windows {
			if c == '\\' {
				start := i
				for i+1 < len(chars) && chars[i+1] == '\\' {
					i++
				}
				count := i - start + 1
				if i+1 < len(chars) && chars[i+1] == '"' {
					word.WriteString(strings.Repeat("\\", count/2))
					if count%2 == 1 {
						word.WriteRune('"')
						i++
					}
				} else {
					word.WriteString(strings.Repeat("\\", count))
				}
				inWord = true
				continue
			}
			if c == '"' {
				inWord = true
				if quote != 0 && i+1 < len(chars) && chars[i+1] == '"' {
					word.WriteRune('"')
					i++
				} else if quote == 0 {
					quote = '"'
				} else {
					quote = 0
				}
				continue
			}
		} else {
			if c == '\\' && quote != '\'' {
				if i+1 == len(chars) {
					return nil, fmt.Errorf("the command ends with an incomplete escape")
				}
				next := chars[i+1]
				if quote == '"' && !strings.ContainsRune("$`\"\\\n", next) {
					word.WriteRune(c)
					inWord = true
					continue
				}
				i++
				if next != '\n' {
					word.WriteRune(next)
					inWord = true
				}
				continue
			}
			if c == '\'' || c == '"' {
				if quote == 0 {
					quote = c
					inWord = true
					continue
				}
				if quote == c {
					quote = 0
					continue
				}
			}
		}
		if unicode.IsSpace(c) && quote == 0 {
			if inWord {
				words = append(words, word.String())
				word.Reset()
				inWord = false
			}
		} else {
			word.WriteRune(c)
			inWord = true
		}
	}
	if quote != 0 {
		return nil, fmt.Errorf("the command contains an unclosed quote")
	}
	if inWord {
		words = append(words, word.String())
	}
	return words, nil
}
