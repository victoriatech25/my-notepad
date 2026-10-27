# 빌드 규칙
CC := gcc
CFLAGS ?= -O2 -Wall
OBJ = main.o util.o

.PHONY: all clean

all: app

app: $(OBJ)
	$(CC) $(CFLAGS) -o $@ $(OBJ)

ifeq ($(DEBUG),1)
CFLAGS += -g
endif

clean:
	rm -f app *.o
