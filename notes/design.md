# actor-x1

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
