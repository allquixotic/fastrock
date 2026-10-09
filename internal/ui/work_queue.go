package ui

import (
	"context"
	"errors"
)

var errWorkQueueFull = errors.New("Too many background operations. Wait for the current work, then retry.")

type workTask struct {
	run      func()
	rejected []func()
}

func (a *App) startWorkers() {
	a.workOnce.Do(func() {
		a.readJobs = make(chan workTask, 64)
		a.writeJobs = make(chan workTask, 16)
		a.controlJobs = make(chan workTask, 32)
		for _, lane := range []struct {
			jobs    chan workTask
			workers int
		}{{a.readJobs, 6}, {a.writeJobs, 2}, {a.controlJobs, 2}} {
			for range lane.workers {
				go func() {
					for {
						select {
						case task := <-lane.jobs:
							if a.ctx.Err() == nil {
								a.safeWork(func() {
									defer func() {
										if failure := recover(); failure != nil {
											a.post(func() {
												for _, failed := range task.rejected {
													if failed != nil {
														failed()
													}
												}
											})
											panic(failure)
										}
									}()
									task.run()
								})
							}
						case <-a.ctx.Done():
							return
						}
					}
				}()
			}
		}
	})
}

func (a *App) work(f func(), rejected ...func()) bool { return a.enqueueWork(0, f, rejected...) }
func (a *App) writeWork(f func(), rejected ...func()) bool {
	return a.enqueueWork(1, f, rejected...)
}
func (a *App) controlWork(f func(), rejected ...func()) bool { return a.enqueueWork(2, f, rejected...) }
func (a *App) enqueueWork(lane int, f func(), rejected ...func()) bool {
	if a.ctx == nil {
		a.ctx = context.Background()
	}
	a.startWorkers()
	jobs := a.readJobs
	if lane == 1 {
		jobs = a.writeJobs
	}
	if lane == 2 {
		jobs = a.controlJobs
	}
	select {
	case <-a.ctx.Done():
		return false
	default:
	}
	select {
	case jobs <- workTask{run: f, rejected: rejected}:
		return true
	default:
		a.post(func() {
			a.report(errWorkQueueFull)
			for _, failed := range rejected {
				if failed != nil {
					failed()
				}
			}
		})
		return false
	}
}
