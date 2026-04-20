# actor-x1

## Overview

Actor-x1 is a system that uses Communicating
Sequential Process (CSP) invented by Tony Hoare
In 1978 and is commonly termed an actor model.

 - There will be a management system (MS) that:

   - Provides the capability to interact with
     the runtime providing control and to capture
     logs and assists in developing the app
   - Creates an hello world scaffolding
   - provides discovery capabilities of other
     other actors.

 - A runtime that drives the system and provides
   resources and services for the system
   - interfaces between MS and the application
 
 - One or more actors running on one or more
   threads. Each actor adheres to these
   invariants:
   - Has on basic entry point; handler()
   - The handler() handles one message at a time and
     must run to completion as quickly as possible.
   - The handler() never ever blocks
   - If a handler() has tasks that take to much time
     a technique that can be used is to spawn a
     threads to do work. That thread would then use
     messages to communicate with its parent and
     other actors as it does its work.

## Future capabilities
   - I would like Actors to be implemented as an HSM.
     This potentially allows an Actor to implement
     sophisticated algorithms and prove correct operation.

## Stage1 runtime

Design and implement the simplest runtime possible

### Message

struct Message {}

### Actor

Actor implements an Actor trait

trait Actor {
  // Handles a message 
  fn handle_message(msg: Message)
}

### Goal1

- Run two actors on one thread.
- A single message which is an empty struct
- A single queue between the two actors.
- Message ping pongs between the two actors
  for a time in seconds specified as a f64
  in seconds from the command line

## Goal2

- Same as Goal1 but each actor on a separate thread.

## Stage2 runtime

We will design and implement the slightly more capable runtime

It supports 3 actors A, B and C

### Message

struct Message {
  src_id: u32,
  dst_id: u32,
  send_count: u64,
}

### Actor

Actor implements an Actor trait

trait Actor {
  // Create a new actor
  fn new(id: u32, name: &str) -> Self;

  // Return the actors name
  fn name(&Self) -> &str;

  // Handle a message
  fn handle_message(msg: mut Message);
}

### Goal1

- Run 3 actors on one thread.
- Runtime "sends" 5 messages each to a random destination actor
- Each actor forwards the message they receive to a random
  dst_id that they select.

### Goal2
- Same as Goal1 but each actor on a separate thread.

